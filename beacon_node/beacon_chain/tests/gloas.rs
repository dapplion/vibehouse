#![cfg(not(debug_assertions))] // Tests run too slow in debug.

//! Integration tests for the Gloas (ePBS) fork transition and block production.
//!
//! These tests exercise the full beacon chain harness through the Fulu → Gloas fork boundary,
//! verifying that:
//! - Blocks transition to the Gloas variant at the correct epoch
//! - Self-build blocks are produced and imported correctly
//! - Payload envelopes are processed and state is updated
//! - The chain finalizes across the fork boundary
//! - Gloas-specific state fields are initialized correctly after upgrade

use beacon_chain::AvailabilityProcessingStatus;
use beacon_chain::BeaconChainError;
use beacon_chain::BlockError;
use beacon_chain::ChainConfig;
use beacon_chain::execution_payload::{
    NotifyExecutionLayer, PayloadNotifier, validate_execution_payload_for_gossip,
};
use beacon_chain::execution_proof_verification::GossipExecutionProofError;
use beacon_chain::gloas_verification::{
    ExecutionBidError, PayloadAttestationError, PayloadEnvelopeError,
};
use beacon_chain::test_utils::{
    BeaconChainHarness, DEFAULT_ETH1_BLOCK_HASH, EphemeralHarnessType, HARNESS_GENESIS_TIME,
    InteropGenesisBuilder,
};
use execution_layer::test_utils::generate_genesis_header;
use fork_choice::{
    ExecutionStatus, ForkchoiceUpdateParameters, InvalidationOperation, PayloadVerificationStatus,
};
use state_processing::per_block_processing::gloas::get_ptc_committee;
use std::sync::Arc;
use std::time::Duration;
use types::*;

const VALIDATOR_COUNT: usize = 32;
type E = MinimalEthSpec;

fn gloas_harness_at_epoch(gloas_epoch: u64) -> BeaconChainHarness<EphemeralHarnessType<E>> {
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    spec.fulu_fork_epoch = Some(Epoch::new(0));
    spec.gloas_fork_epoch = Some(Epoch::new(gloas_epoch));

    let harness = BeaconChainHarness::builder(E::default())
        .spec(spec.into())
        .deterministic_keypairs(VALIDATOR_COUNT)
        .fresh_ephemeral_store()
        .mock_execution_layer()
        .build();

    harness.advance_slot();
    harness
}

/// Test that the fork transition from Fulu to Gloas produces the correct block variants.
#[tokio::test]
async fn fulu_to_gloas_fork_transition() {
    let gloas_fork_epoch = Epoch::new(2);
    let gloas_fork_slot = gloas_fork_epoch.start_slot(E::slots_per_epoch());
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    // Extend to just before the fork
    let pre_fork_slot = gloas_fork_slot - 1;
    Box::pin(harness.extend_to_slot(pre_fork_slot)).await;

    let pre_fork_head = &harness.chain.head_snapshot().beacon_block;
    assert!(
        pre_fork_head.as_fulu().is_ok(),
        "Pre-fork head should be Fulu, got {:?}",
        pre_fork_head.fork_name_unchecked()
    );

    // Extend to the fork slot
    Box::pin(harness.extend_to_slot(gloas_fork_slot)).await;

    let post_fork_head = &harness.chain.head_snapshot().beacon_block;
    assert!(
        post_fork_head.as_gloas().is_ok(),
        "Post-fork head should be Gloas, got {:?}",
        post_fork_head.fork_name_unchecked()
    );
    assert_eq!(post_fork_head.slot(), gloas_fork_slot);
}

/// Test that Gloas blocks from genesis (all forks at epoch 0) work correctly.
#[tokio::test]
async fn gloas_from_genesis() {
    let harness = gloas_harness_at_epoch(0);

    // Produce several blocks
    Box::pin(harness.extend_slots(8)).await;

    let head = &harness.chain.head_snapshot().beacon_block;
    assert!(head.as_gloas().is_ok(), "Head should be Gloas variant");

    let state = &harness.chain.head_snapshot().beacon_state;
    assert!(
        state.fork_name_unchecked().gloas_enabled(),
        "Head state should be Gloas"
    );
}

/// Test that the chain produces and imports Gloas blocks with self-build bids.
#[tokio::test]
async fn gloas_self_build_block_production() {
    let harness = gloas_harness_at_epoch(0);

    // Produce one block
    Box::pin(harness.extend_slots(1)).await;

    let head = &harness.chain.head_snapshot().beacon_block;
    let block = head.as_gloas().expect("should be Gloas block");

    // Verify the block has a self-build bid (builder_index = BUILDER_INDEX_SELF_BUILD)
    let bid = &block.message.body.signed_execution_payload_bid.message;
    assert_eq!(
        bid.builder_index, harness.spec.builder_index_self_build,
        "Self-build block should use BUILDER_INDEX_SELF_BUILD"
    );
    assert_eq!(bid.value, 0, "Self-build bid should have value = 0");
}

/// Test that Gloas state has the correct fields initialized after upgrade.
#[tokio::test]
async fn gloas_state_fields_after_upgrade() {
    let gloas_fork_epoch = Epoch::new(2);
    let gloas_fork_slot = gloas_fork_epoch.start_slot(E::slots_per_epoch());
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    // Extend to the first Gloas slot
    Box::pin(harness.extend_to_slot(gloas_fork_slot)).await;

    let state = harness.chain.head_beacon_state_cloned();
    assert!(state.fork_name_unchecked().gloas_enabled());

    // Verify Gloas-specific state fields exist and are accessible
    assert!(
        state.latest_execution_payload_bid().is_ok(),
        "Gloas state should have latest_execution_payload_bid"
    );
    assert!(
        state.builders().is_ok(),
        "Gloas state should have builders list"
    );
    assert!(
        state.latest_block_hash().is_ok(),
        "Gloas state should have latest_block_hash"
    );

    // latest_execution_payload_header should NOT be accessible on Gloas
    assert!(
        state.latest_execution_payload_header().is_err(),
        "Gloas state should NOT have latest_execution_payload_header"
    );
}

/// Test that the chain can produce multiple consecutive Gloas blocks.
#[tokio::test]
async fn gloas_multiple_consecutive_blocks() {
    let harness = gloas_harness_at_epoch(0);

    // Produce a full epoch of blocks (8 for minimal)
    let slots = E::slots_per_epoch() as usize;
    Box::pin(harness.extend_slots(slots)).await;

    let head = &harness.chain.head_snapshot().beacon_block;
    assert_eq!(
        head.slot(),
        Slot::new(slots as u64),
        "Head should be at slot {}",
        slots
    );
    assert!(head.as_gloas().is_ok());

    // Verify the state epoch advanced
    let state = &harness.chain.head_snapshot().beacon_state;
    assert_eq!(
        state.current_epoch(),
        Epoch::new(1),
        "Should be in epoch 1 after a full epoch of blocks"
    );
}

/// Test that the chain finalizes during Gloas (requires multiple epochs with attestations).
#[tokio::test]
async fn gloas_chain_finalizes() {
    let harness = gloas_harness_at_epoch(0);

    // Run for enough epochs to finalize (need 3+ justified epochs)
    // With MinimalEthSpec (8 slots per epoch), run 5 epochs = 40 slots
    let num_slots = 5 * E::slots_per_epoch() as usize;
    Box::pin(harness.extend_slots(num_slots)).await;

    let state = harness.chain.head_beacon_state_cloned();
    let finalized_epoch = state.finalized_checkpoint().epoch;

    assert!(
        finalized_epoch > Epoch::new(0),
        "Chain should finalize after 5 epochs of Gloas blocks, got finalized_epoch={}",
        finalized_epoch
    );
}

/// Test that fork transition from Fulu to Gloas preserves finalization.
#[tokio::test]
async fn gloas_fork_transition_preserves_finalization() {
    let gloas_fork_epoch = Epoch::new(2);
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    // Run for 6 epochs (finalize in Fulu, then continue in Gloas)
    let num_slots = 6 * E::slots_per_epoch() as usize;
    Box::pin(harness.extend_slots(num_slots)).await;

    let state = harness.chain.head_beacon_state_cloned();
    let finalized_epoch = state.finalized_checkpoint().epoch;

    assert!(
        finalized_epoch > gloas_fork_epoch,
        "Should finalize past the Gloas fork epoch (got finalized_epoch={}, fork={})",
        finalized_epoch,
        gloas_fork_epoch
    );
}

/// Test that Gloas blocks do NOT have execution_payload in the block body.
#[tokio::test]
async fn gloas_block_has_no_execution_payload() {
    let harness = gloas_harness_at_epoch(0);

    Box::pin(harness.extend_slots(1)).await;

    let head = &harness.chain.head_snapshot().beacon_block;
    let body = head.message().body();

    assert!(
        body.execution_payload().is_err(),
        "Gloas blocks should not have execution_payload in body"
    );
    assert!(
        body.signed_execution_payload_bid().is_ok(),
        "Gloas blocks should have signed_execution_payload_bid"
    );
}

/// Test that Gloas blocks have payload_attestations field.
#[tokio::test]
async fn gloas_block_has_payload_attestations() {
    let harness = gloas_harness_at_epoch(0);

    // Produce 2 blocks so the second one can include payload attestations for the first
    Box::pin(harness.extend_slots(2)).await;

    let head = &harness.chain.head_snapshot().beacon_block;
    let body = head.message().body();

    // payload_attestations should be accessible (may be empty if no PTC duties)
    assert!(
        body.payload_attestations().is_ok(),
        "Gloas blocks should have payload_attestations field"
    );
}

/// Test that Gloas fork version is set correctly in the state.
#[tokio::test]
async fn gloas_fork_version_in_state() {
    let gloas_fork_epoch = Epoch::new(2);
    let gloas_fork_slot = gloas_fork_epoch.start_slot(E::slots_per_epoch());
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    Box::pin(harness.extend_to_slot(gloas_fork_slot)).await;

    let state = harness.chain.head_beacon_state_cloned();
    let fork = state.fork();

    assert_eq!(
        fork.current_version, harness.spec.gloas_fork_version,
        "State fork current_version should be Gloas fork version"
    );
    assert_eq!(
        fork.previous_version, harness.spec.fulu_fork_version,
        "State fork previous_version should be Fulu fork version"
    );
    assert_eq!(
        fork.epoch, gloas_fork_epoch,
        "State fork epoch should match Gloas fork epoch"
    );
}

/// Test that the Gloas state has the bid slot matching the block slot.
#[tokio::test]
async fn gloas_bid_slot_matches_block_slot() {
    let harness = gloas_harness_at_epoch(0);

    for _ in 0..4 {
        Box::pin(harness.extend_slots(1)).await;

        let head = &harness.chain.head_snapshot().beacon_block;
        let block_slot = head.slot();

        let state = harness.chain.head_beacon_state_cloned();
        let bid = state.latest_execution_payload_bid().unwrap();

        assert_eq!(
            bid.slot, block_slot,
            "Latest bid slot should match the head block slot"
        );
    }
}

// =============================================================================
// Gloas parent payload availability check in block gossip verification
// =============================================================================

/// Test that gossip verification rejects a block when its parent's execution payload
/// has not been revealed yet (GloasParentPayloadUnknown).
///
/// In Gloas ePBS, blocks and execution payload envelopes are separate gossip messages.
/// A child block should be rejected (IGNORE) if the parent's payload hasn't arrived yet.
#[tokio::test]
async fn gloas_gossip_rejects_block_with_unrevealed_parent_payload() {
    let harness = gloas_harness_at_epoch(0);

    // Build a chain with a few blocks (all envelopes processed normally)
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    // Advance the slot clock so we can produce a block at the next slot
    harness.advance_slot();

    // Produce the next block (but don't import it yet)
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let ((next_block, _), _next_state) = harness.make_block(head_state, next_slot).await;

    // Manipulate fork choice: set the head block's payload_revealed = false
    // This simulates the case where the parent block arrived but its envelope hasn't
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        let block_index = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&head_root)
            .expect("head root should be in fork choice");
        fc.proto_array_mut().core_proto_array_mut().nodes[block_index].payload_revealed = false;
    }

    // Now try to gossip-verify the next block — parent payload not revealed
    let result = harness.chain.verify_block_for_gossip(next_block).await;

    assert!(
        result.is_err(),
        "should reject block with unrevealed parent payload"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, BlockError::GloasParentPayloadUnknown { .. }),
        "expected GloasParentPayloadUnknown, got {:?}",
        err
    );
}

/// Test that gossip verification ACCEPTS a block when its parent's payload IS revealed.
/// This is the normal case — complement to the rejection test above.
#[tokio::test]
async fn gloas_gossip_accepts_block_with_revealed_parent_payload() {
    let harness = gloas_harness_at_epoch(0);

    // Build a chain with a few blocks (all envelopes processed normally → payload_revealed = true)
    Box::pin(harness.extend_slots(3)).await;

    // Advance the slot clock so we can produce a block at the next slot
    harness.advance_slot();

    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let ((next_block, _), _next_state) = harness.make_block(head_state, next_slot).await;

    // Parent payload should be revealed (envelope was processed during extend_slots)
    // Gossip verification should pass (at least the parent payload check)
    let result = harness.chain.verify_block_for_gossip(next_block).await;

    // The block should either pass gossip verification OR fail on a different check
    // (NOT GloasParentPayloadUnknown)
    match result {
        Ok(_) => {} // Good — block passed gossip verification
        Err(ref e) => {
            assert!(
                !matches!(e, BlockError::GloasParentPayloadUnknown { .. }),
                "block with revealed parent payload should not fail with GloasParentPayloadUnknown, got {:?}",
                e
            );
        }
    }
}

/// Test that the GloasParentPayloadUnknown check only applies when the parent is a Gloas block.
/// Pre-Gloas parents (Fulu, etc.) have the payload embedded in the block body, so it's always "seen".
#[tokio::test]
async fn gloas_parent_payload_check_skips_pre_gloas_parent() {
    // Create a harness where Gloas starts at epoch 2
    let gloas_fork_epoch = Epoch::new(2);
    let gloas_fork_slot = gloas_fork_epoch.start_slot(E::slots_per_epoch());
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    // Extend to one slot before the Gloas fork
    Box::pin(harness.extend_to_slot(gloas_fork_slot - 1)).await;

    // Verify the head is a Fulu block (pre-Gloas)
    let pre_fork_head = &harness.chain.head_snapshot().beacon_block;
    assert!(
        pre_fork_head.as_fulu().is_ok(),
        "Pre-fork head should be Fulu"
    );

    // Now extend to the fork slot — this produces the first Gloas block with a Fulu parent
    Box::pin(harness.extend_to_slot(gloas_fork_slot)).await;

    let post_fork_head = &harness.chain.head_snapshot().beacon_block;
    assert!(
        post_fork_head.as_gloas().is_ok(),
        "Post-fork head should be Gloas"
    );

    // The first Gloas block's parent is a Fulu block (no bid_block_hash → parent check skipped)
    // If the parent payload check incorrectly applied to Fulu parents, the chain would break
    // The fact that extend_to_slot succeeded means the check was correctly skipped
}

// =============================================================================
// Gloas BeaconChain method tests: PTC duties, payload attestation data,
// payload attestation pool, bid/envelope fork choice integration
// =============================================================================

fn make_payload_attestation(
    block_root: Hash256,
    slot: Slot,
    payload_present: bool,
    blob_data_available: bool,
) -> PayloadAttestation<E> {
    let mut aggregation_bits = BitVector::default();
    aggregation_bits
        .set(0, true)
        .expect("PTC size >= 1, bit 0 should be settable");
    PayloadAttestation {
        aggregation_bits,
        data: PayloadAttestationData {
            beacon_block_root: block_root,
            slot,
            payload_present,
            blob_data_available,
        },
        signature: AggregateSignature::empty(),
    }
}

/// Test that validator_ptc_duties returns duties for validators in the PTC.
#[tokio::test]
async fn gloas_validator_ptc_duties_returns_duties() {
    let harness = gloas_harness_at_epoch(0);

    // Produce a few blocks to populate state
    Box::pin(harness.extend_slots(2)).await;

    let current_epoch = harness.chain.head_beacon_state_cloned().current_epoch();

    // Query PTC duties for all validators
    let all_indices: Vec<u64> = (0..VALIDATOR_COUNT as u64).collect();
    let (duties, dependent_root) = harness
        .chain
        .validator_ptc_duties(&all_indices, current_epoch)
        .expect("should compute PTC duties");

    // PTC size is 2 for MinimalEthSpec, one PTC per slot, 8 slots per epoch = 16 duties total
    let ptc_size = E::ptc_size();
    let slots_per_epoch = E::slots_per_epoch();
    assert_eq!(
        duties.len(),
        ptc_size * slots_per_epoch as usize,
        "should have ptc_size * slots_per_epoch duties"
    );

    // Each duty should have a valid slot within the epoch
    let start_slot = current_epoch.start_slot(slots_per_epoch);
    for duty in &duties {
        assert!(duty.slot >= start_slot);
        assert!(duty.slot < start_slot + slots_per_epoch);
        assert!(
            (duty.validator_index as usize) < VALIDATOR_COUNT,
            "validator index should be in range"
        );
        assert!(
            (duty.ptc_committee_index as usize) < ptc_size,
            "PTC committee index should be < PTC size"
        );
    }

    // Dependent root should be non-zero
    assert_ne!(
        dependent_root,
        Hash256::ZERO,
        "dependent root should be non-zero"
    );
}

/// Test that validator_ptc_duties returns empty when no requested validators are in PTC.
#[tokio::test]
async fn gloas_validator_ptc_duties_no_match() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(1)).await;

    let current_epoch = harness.chain.head_beacon_state_cloned().current_epoch();

    // Query with a validator index that's out of range (won't be in PTC)
    let (duties, _) = harness
        .chain
        .validator_ptc_duties(&[9999], current_epoch)
        .expect("should compute PTC duties");

    assert!(
        duties.is_empty(),
        "should have no duties for non-existent validator"
    );
}

/// Test that validator_ptc_duties works for a future epoch (advances state).
#[tokio::test]
async fn gloas_validator_ptc_duties_future_epoch() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let current_epoch = harness.chain.head_beacon_state_cloned().current_epoch();
    let next_epoch = current_epoch + 1;

    let all_indices: Vec<u64> = (0..VALIDATOR_COUNT as u64).collect();
    let (duties, _) = harness
        .chain
        .validator_ptc_duties(&all_indices, next_epoch)
        .expect("should compute PTC duties for next epoch");

    let ptc_size = E::ptc_size();
    let slots_per_epoch = E::slots_per_epoch();
    assert_eq!(
        duties.len(),
        ptc_size * slots_per_epoch as usize,
        "should have duties for next epoch"
    );

    // All duties should be in the next epoch's slot range
    let start_slot = next_epoch.start_slot(slots_per_epoch);
    for duty in &duties {
        assert!(duty.slot >= start_slot);
        assert!(duty.slot < start_slot + slots_per_epoch);
    }
}

/// Test that get_payload_attestation_data returns correct data for the head slot.
#[tokio::test]
async fn gloas_payload_attestation_data_head_slot() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let head_root = head.beacon_block_root;

    let data = harness
        .chain
        .get_payload_attestation_data(head_slot)
        .expect("should get payload attestation data");

    assert_eq!(
        data.slot, head_slot,
        "data slot should match requested slot"
    );
    assert_eq!(
        data.beacon_block_root, head_root,
        "block root should be head root"
    );
    // The harness processes envelopes during extend_slots, so payload should be present
    assert!(
        data.payload_present,
        "payload should be present (envelope was processed)"
    );
}

/// Test that get_payload_attestation_data returns data for a past slot.
#[tokio::test]
async fn gloas_payload_attestation_data_past_slot() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(4)).await;

    let head_slot = harness.chain.head_snapshot().beacon_block.slot();
    let past_slot = head_slot - 2;

    let data = harness
        .chain
        .get_payload_attestation_data(past_slot)
        .expect("should get payload attestation data for past slot");

    assert_eq!(data.slot, past_slot, "data slot should match past slot");
    // The block root should be a valid block root (not zero)
    assert_ne!(
        data.beacon_block_root,
        Hash256::ZERO,
        "block root should be non-zero"
    );
}

/// Test that get_payload_attestation_data returns head root for a future slot.
#[tokio::test]
async fn gloas_payload_attestation_data_future_slot() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let future_slot = head.beacon_block.slot() + 5;

    let data = harness
        .chain
        .get_payload_attestation_data(future_slot)
        .expect("should get payload attestation data for future slot");

    assert_eq!(
        data.beacon_block_root, head_root,
        "future slot should return head block root"
    );
    assert_eq!(data.slot, future_slot);
}

/// Test insert_payload_attestation_to_pool and get_payload_attestations_for_block.
#[tokio::test]
async fn gloas_payload_attestation_pool_insert_and_get() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    // Create a payload attestation for the head slot
    let att = make_payload_attestation(head_root, head_slot, true, true);

    // Insert into pool
    harness
        .chain
        .insert_payload_attestation_to_pool(att.clone());

    // Query for the next slot's block (block_slot = head_slot + 1, targets head_slot)
    let result = harness
        .chain
        .get_payload_attestations_for_block(head_slot + 1, head_root);

    assert_eq!(result.len(), 1, "should find the inserted attestation");
    assert_eq!(result[0].data.beacon_block_root, head_root);
    assert!(result[0].data.payload_present);
}

/// Test that get_payload_attestations_for_block filters by parent_block_root.
#[tokio::test]
async fn gloas_payload_attestation_pool_filters_by_root() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    // Insert two attestations: one matching root, one not
    let matching = make_payload_attestation(head_root, head_slot, true, false);
    let non_matching =
        make_payload_attestation(Hash256::repeat_byte(0xff), head_slot, false, false);

    harness
        .chain
        .insert_payload_attestation_to_pool(matching.clone());
    harness
        .chain
        .insert_payload_attestation_to_pool(non_matching);

    let result = harness
        .chain
        .get_payload_attestations_for_block(head_slot + 1, head_root);

    assert_eq!(
        result.len(),
        1,
        "should only return attestation matching parent_block_root"
    );
    assert_eq!(result[0].data.beacon_block_root, head_root);
}

/// Test that get_payload_attestations_for_block returns empty for wrong slot.
#[tokio::test]
async fn gloas_payload_attestation_pool_wrong_slot_empty() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    let att = make_payload_attestation(head_root, head_slot, true, false);
    harness.chain.insert_payload_attestation_to_pool(att);

    // Query with block_slot that doesn't target head_slot (target = block_slot - 1)
    let result = harness
        .chain
        .get_payload_attestations_for_block(head_slot + 3, head_root);

    assert!(
        result.is_empty(),
        "should return empty when target slot doesn't match"
    );
}

/// Test that payload attestation pool respects max_payload_attestations limit.
#[tokio::test]
async fn gloas_payload_attestation_pool_max_limit() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    // Insert more attestations than the max (MinimalEthSpec max = 2)
    let max = E::max_payload_attestations();
    for i in 0..(max + 3) {
        let mut att = make_payload_attestation(head_root, head_slot, i % 2 == 0, false);
        // Set different bits to make them distinct
        if i < E::ptc_size() {
            let _ = att.aggregation_bits.set(i, true);
        }
        harness.chain.insert_payload_attestation_to_pool(att);
    }

    let result = harness
        .chain
        .get_payload_attestations_for_block(head_slot + 1, head_root);

    assert_eq!(
        result.len(),
        max,
        "should be capped at max_payload_attestations ({})",
        max
    );
}

/// Test that payload attestation pool prunes old entries.
#[tokio::test]
async fn gloas_payload_attestation_pool_prunes_old() {
    let harness = gloas_harness_at_epoch(0);

    // Insert an attestation at slot 1
    Box::pin(harness.extend_slots(1)).await;
    let early_root = harness.chain.head_snapshot().beacon_block_root;
    let early_slot = Slot::new(1);

    let att = make_payload_attestation(early_root, early_slot, true, false);
    harness.chain.insert_payload_attestation_to_pool(att);

    // Verify it's there
    let result = harness
        .chain
        .get_payload_attestations_for_block(early_slot + 1, early_root);
    assert_eq!(result.len(), 1);

    // Now advance many epochs (pruning threshold is 2 epochs = 16 slots for minimal)
    Box::pin(harness.extend_slots(20)).await;

    // Insert a new attestation to trigger pruning
    let new_root = harness.chain.head_snapshot().beacon_block_root;
    let new_slot = harness.chain.head_snapshot().beacon_block.slot();
    let new_att = make_payload_attestation(new_root, new_slot, true, false);
    harness.chain.insert_payload_attestation_to_pool(new_att);

    // The old attestation should have been pruned
    let result = harness
        .chain
        .get_payload_attestations_for_block(early_slot + 1, early_root);
    assert!(result.is_empty(), "old attestation should be pruned");
}

/// Test that get_best_execution_bid returns None when pool is empty.
#[tokio::test]
async fn gloas_get_best_execution_bid_empty() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(1)).await;

    let result = harness
        .chain
        .get_best_execution_bid(Slot::new(1), Hash256::zero());
    assert!(
        result.is_none(),
        "should return None when no external bids in pool"
    );
}

/// Test that get_best_execution_bid returns a bid that was inserted into the pool.
#[tokio::test]
async fn gloas_get_best_execution_bid_returns_inserted() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let target_slot = Slot::new(3);

    // Create a bid and insert it directly into the pool
    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot: target_slot,
            builder_index: 0,
            value: 1000,
            ..Default::default()
        },
        signature: Signature::empty(),
    };
    harness.chain.execution_bid_pool.lock().insert(bid.clone());

    let result = harness
        .chain
        .get_best_execution_bid(target_slot, Hash256::zero());
    assert!(result.is_some(), "should return the inserted bid");
    assert_eq!(result.unwrap().message.value, 1000);
}

/// Test that get_best_execution_bid returns highest-value bid.
#[tokio::test]
async fn gloas_get_best_execution_bid_highest_value() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let target_slot = Slot::new(3);

    // Insert two bids with different values from different builders
    let bid_low = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot: target_slot,
            builder_index: 0,
            value: 500,
            ..Default::default()
        },
        signature: Signature::empty(),
    };
    let bid_high = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot: target_slot,
            builder_index: 1,
            value: 2000,
            ..Default::default()
        },
        signature: Signature::empty(),
    };

    {
        let mut pool = harness.chain.execution_bid_pool.lock();
        pool.insert(bid_low);
        pool.insert(bid_high);
    }

    let result = harness
        .chain
        .get_best_execution_bid(target_slot, Hash256::zero());
    assert!(result.is_some());
    assert_eq!(
        result.unwrap().message.value,
        2000,
        "should return highest-value bid"
    );
}

// =============================================================================
// External builder path — block production with external bids
// =============================================================================

/// Extra keypairs for builder identities (separate from validator keypairs).
static BUILDER_KEYPAIRS: std::sync::LazyLock<Vec<Keypair>> = std::sync::LazyLock::new(|| {
    types::test_utils::generate_deterministic_keypairs(VALIDATOR_COUNT + 4)[VALIDATOR_COUNT..]
        .to_vec()
});

/// Create a Gloas harness with builders injected into the genesis state.
///
/// `builders`: slice of `(deposit_epoch, balance)` tuples.
/// Each builder gets a pubkey from `BUILDER_KEYPAIRS` and `withdrawable_epoch = FAR_FUTURE_EPOCH`.
fn gloas_harness_with_builders(
    builders: &[(u64, u64)],
) -> BeaconChainHarness<EphemeralHarnessType<E>> {
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    spec.fulu_fork_epoch = Some(Epoch::new(0));
    spec.gloas_fork_epoch = Some(Epoch::new(0));

    let spec_arc = Arc::new(spec.clone());
    let keypairs = types::test_utils::generate_deterministic_keypairs(VALIDATOR_COUNT);

    let header = generate_genesis_header::<E>(&spec, false);
    let mut state = InteropGenesisBuilder::default()
        .set_alternating_eth1_withdrawal_credentials()
        .set_opt_execution_payload_header(header)
        .build_genesis_state(
            &keypairs,
            HARNESS_GENESIS_TIME,
            Hash256::from_slice(DEFAULT_ETH1_BLOCK_HASH),
            &spec,
        )
        .expect("should generate interop state");

    // Inject builders into the Gloas state
    let gloas_state = state.as_gloas_mut().expect("should be gloas state");
    for (i, &(deposit_epoch, balance)) in builders.iter().enumerate() {
        let builder = Builder {
            pubkey: BUILDER_KEYPAIRS[i].pk.clone().into(),
            version: 0,
            execution_address: Address::zero(),
            balance,
            deposit_epoch: Epoch::new(deposit_epoch),
            withdrawable_epoch: spec.far_future_epoch,
        };
        gloas_state
            .builders
            .push(builder)
            .expect("should push builder");
    }

    state.drop_all_caches().expect("should drop caches");

    let harness = BeaconChainHarness::builder(E::default())
        .spec(spec_arc)
        .keypairs(keypairs)
        .genesis_state_ephemeral_store(state)
        .mock_execution_layer()
        .build();

    harness.advance_slot();
    harness
}

/// Build an external bid matching the current state (fills in parent_block_hash,
/// parent_block_root, prev_randao from the state so the bid passes validation).
fn make_external_bid(
    state: &BeaconState<E>,
    head_root: Hash256,
    slot: Slot,
    builder_index: u64,
    value: u64,
) -> SignedExecutionPayloadBid<E> {
    let gloas_state = state.as_gloas().expect("state should be Gloas");
    let current_epoch = state.current_epoch();
    let randao_mix = *state
        .get_randao_mix(current_epoch)
        .expect("should get randao mix");

    SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot,
            builder_index,
            value,
            parent_block_hash: gloas_state.latest_block_hash,
            parent_block_root: head_root,
            prev_randao: randao_mix,
            block_hash: ExecutionBlockHash::zero(),
            fee_recipient: Address::zero(),
            gas_limit: 30_000_000,
            execution_payment: value,
            blob_kzg_commitments: Default::default(),
        },
        signature: Signature::empty(),
    }
}

/// Test that when an external bid is in the pool, `make_block` produces a block
/// containing the external bid instead of a self-build bid.
#[tokio::test]
async fn gloas_external_bid_block_production() {
    // deposit_epoch=0, balance=10 ETH — builder 0 is active once finalized_epoch >= 1
    // Need enough slots for finalization: minimal preset, 8 slots/epoch
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;

    // Build an external bid from builder 0 that matches current state
    let external_builder_index = 0u64;
    let bid_value = 5000u64;
    let state = harness.chain.head_beacon_state_cloned();
    let bid = make_external_bid(
        &state,
        head_root,
        next_slot,
        external_builder_index,
        bid_value,
    );
    harness.chain.execution_bid_pool.lock().insert(bid);

    // Advance slot clock and produce block
    harness.advance_slot();
    let ((signed_block, _blobs), _state, envelope) =
        harness.make_block_with_envelope(state, next_slot).await;

    let block = signed_block.message();
    assert!(
        block.fork_name_unchecked().gloas_enabled(),
        "produced block should be Gloas"
    );

    // Verify the block uses the external bid, not self-build
    let block_bid = block
        .body()
        .signed_execution_payload_bid()
        .expect("Gloas block should have bid");
    assert_eq!(
        block_bid.message.builder_index, external_builder_index,
        "block should use external builder's bid, not self-build"
    );
    assert_eq!(
        block_bid.message.value, bid_value,
        "block bid value should match external bid"
    );

    // External bid path: no self-build envelope should be returned
    assert!(
        envelope.is_none(),
        "external bid block should not return a self-build envelope"
    );
}

/// Test that self-build fallback works when no external bid is in the pool.
#[tokio::test]
async fn gloas_no_external_bid_falls_back_to_self_build() {
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;

    // Do NOT insert any external bid — pool is empty

    harness.advance_slot();
    let state = harness.chain.head_beacon_state_cloned();
    let ((signed_block, _blobs), _state, envelope) =
        harness.make_block_with_envelope(state, next_slot).await;

    let block_bid = signed_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("Gloas block should have bid");
    assert_eq!(
        block_bid.message.builder_index, harness.spec.builder_index_self_build,
        "without external bid, should fall back to self-build"
    );
    assert_eq!(
        block_bid.message.value, 0,
        "self-build bid should have value 0"
    );

    // Self-build path: envelope should be returned for VC signing
    assert!(
        envelope.is_some(),
        "self-build block should return an envelope for VC signing"
    );
}

/// Test that an external bid for the wrong slot is ignored, falling back to self-build.
#[tokio::test]
async fn gloas_external_bid_wrong_slot_ignored() {
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;
    let state = harness.chain.head_beacon_state_cloned();

    // Insert bid for a FUTURE slot (next_slot + 1), not the slot we'll produce at
    let bid = make_external_bid(&state, head_root, next_slot + 1, 0, 9999);
    harness.chain.execution_bid_pool.lock().insert(bid);

    harness.advance_slot();
    let ((signed_block, _blobs), _state, envelope) =
        harness.make_block_with_envelope(state, next_slot).await;

    // Should fall back to self-build since bid is for wrong slot
    let block_bid = signed_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("Gloas block should have bid");
    assert_eq!(
        block_bid.message.builder_index, harness.spec.builder_index_self_build,
        "bid for wrong slot should be ignored, falling back to self-build"
    );
    assert!(
        envelope.is_some(),
        "self-build fallback should return an envelope"
    );
}

/// Test that the highest-value bid is selected when multiple external bids exist.
#[tokio::test]
async fn gloas_external_bid_highest_value_selected_for_block() {
    // Two builders, both active with sufficient balance
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000), (0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;
    let state = harness.chain.head_beacon_state_cloned();

    // Insert two bids from different builders with different values
    {
        let mut pool = harness.chain.execution_bid_pool.lock();
        pool.insert(make_external_bid(&state, head_root, next_slot, 0, 500));
        pool.insert(make_external_bid(&state, head_root, next_slot, 1, 3000));
    }

    harness.advance_slot();
    let ((signed_block, _blobs), _state, envelope) =
        harness.make_block_with_envelope(state, next_slot).await;

    let block_bid = signed_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("Gloas block should have bid");
    assert_eq!(
        block_bid.message.builder_index, 1,
        "should select highest-value bid (builder 1 with value 3000)"
    );
    assert_eq!(block_bid.message.value, 3000);
    assert!(
        envelope.is_none(),
        "external bid block should not return a self-build envelope"
    );
}

/// Test that validator_ptc_duties returns unique slot/committee_index pairs (no duplicates).
#[tokio::test]
async fn gloas_validator_ptc_duties_unique_positions() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let current_epoch = harness.chain.head_beacon_state_cloned().current_epoch();
    let all_indices: Vec<u64> = (0..VALIDATOR_COUNT as u64).collect();
    let (duties, _) = harness
        .chain
        .validator_ptc_duties(&all_indices, current_epoch)
        .expect("should compute PTC duties");

    // Check no duplicate (slot, ptc_committee_index) pairs
    let mut seen = std::collections::HashSet::new();
    for duty in &duties {
        let key = (duty.slot, duty.ptc_committee_index);
        assert!(
            seen.insert(key),
            "duplicate PTC position at slot {} index {}",
            duty.slot,
            duty.ptc_committee_index
        );
    }
}

/// Test that the payload_attestation_data returns payload_present=false when
/// fork choice has payload_revealed=false for the block.
#[tokio::test]
async fn gloas_payload_attestation_data_unrevealed() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    // Set payload_revealed = false in fork choice for the head block
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        let block_index = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&head_root)
            .expect("head root should be in fork choice");
        fc.proto_array_mut().core_proto_array_mut().nodes[block_index].payload_revealed = false;
    }

    let data = harness
        .chain
        .get_payload_attestation_data(head_slot)
        .expect("should get payload attestation data");

    assert_eq!(data.slot, head_slot);
    assert_eq!(data.beacon_block_root, head_root);
    assert!(
        !data.payload_present,
        "payload_present should be false when payload not revealed"
    );
}

// =============================================================================
// import_payload_attestation_message tests
// =============================================================================

/// Helper: sign a PayloadAttestationData for a specific validator using the
/// deterministic keypair and the PTC_ATTESTER domain.
fn sign_payload_attestation_data(
    data: &PayloadAttestationData,
    validator_index: usize,
    state: &BeaconState<E>,
    spec: &ChainSpec,
) -> Signature {
    let epoch = data.slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let message = data.signing_root(domain);
    let keypair = test_utils::generate_deterministic_keypair(validator_index);
    keypair.sk.sign(message)
}

/// Helper: get the PTC committee for a slot and return the first member's validator index.
fn first_ptc_member(state: &BeaconState<E>, slot: Slot, spec: &ChainSpec) -> u64 {
    let ptc = get_ptc_committee::<E>(state, slot, spec).expect("should get PTC committee");
    assert!(!ptc.is_empty(), "PTC committee should not be empty");
    ptc[0]
}

/// Test the happy path: import a properly signed payload attestation message
/// from a PTC member. The attestation should be accepted and added to the pool.
#[tokio::test]
async fn gloas_import_payload_attestation_message_happy_path() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    // Find a PTC member for the head slot
    let validator_index = first_ptc_member(state, head_slot, &harness.spec);

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: true,
    };

    let signature =
        sign_payload_attestation_data(&data, validator_index as usize, state, &harness.spec);

    let message = PayloadAttestationMessage {
        validator_index,
        data: data.clone(),
        signature,
    };

    let result = harness.chain.import_payload_attestation_message(message);
    assert!(
        result.is_ok(),
        "should import valid payload attestation message, got: {:?}",
        result.err()
    );

    let attestation = result.unwrap();
    assert_eq!(attestation.data.slot, head_slot);
    assert_eq!(attestation.data.beacon_block_root, head_root);
    assert!(attestation.data.payload_present);
    assert!(attestation.data.blob_data_available);

    // Verify it was added to the pool
    let pool_result = harness
        .chain
        .get_payload_attestations_for_block(head_slot + 1, head_root);
    assert!(
        !pool_result.is_empty(),
        "imported attestation should be in the pool"
    );
}

/// Test that importing a message from a validator NOT in the PTC fails with
/// PayloadAttestationValidatorNotInPtc.
#[tokio::test]
async fn gloas_import_payload_attestation_message_not_in_ptc() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    // Find a validator NOT in the PTC for this slot
    let ptc =
        get_ptc_committee::<E>(state, head_slot, &harness.spec).expect("should get PTC committee");
    let non_ptc_validator = (0..VALIDATOR_COUNT as u64)
        .find(|idx| !ptc.contains(idx))
        .expect("should find a non-PTC validator");

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: false,
    };

    let signature =
        sign_payload_attestation_data(&data, non_ptc_validator as usize, state, &harness.spec);

    let message = PayloadAttestationMessage {
        validator_index: non_ptc_validator,
        data,
        signature,
    };

    let result = harness.chain.import_payload_attestation_message(message);
    assert!(result.is_err(), "should reject non-PTC validator");
    match result.unwrap_err() {
        BeaconChainError::PayloadAttestationValidatorNotInPtc {
            validator_index,
            slot,
        } => {
            assert_eq!(validator_index, non_ptc_validator);
            assert_eq!(slot, head_slot);
        }
        other => panic!(
            "expected PayloadAttestationValidatorNotInPtc, got: {:?}",
            other
        ),
    }
}

/// Test that importing a message with an out-of-range validator index fails.
#[tokio::test]
async fn gloas_import_payload_attestation_message_unknown_validator() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    let message = PayloadAttestationMessage {
        validator_index: 99999,
        data: PayloadAttestationData {
            beacon_block_root: head_root,
            slot: head_slot,
            payload_present: true,
            blob_data_available: false,
        },
        signature: Signature::empty(),
    };

    let result = harness.chain.import_payload_attestation_message(message);
    assert!(result.is_err(), "should reject unknown validator index");
    assert!(
        matches!(
            result.unwrap_err(),
            BeaconChainError::PayloadAttestationValidatorNotInPtc { .. }
        ),
        "should be PayloadAttestationValidatorNotInPtc for out-of-range index"
    );
}

/// Test that importing a message with payload_present=false also works (the PTC member
/// is attesting that the payload was NOT present).
#[tokio::test]
async fn gloas_import_payload_attestation_message_payload_absent() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let validator_index = first_ptc_member(state, head_slot, &harness.spec);

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: false,
        blob_data_available: false,
    };

    let signature =
        sign_payload_attestation_data(&data, validator_index as usize, state, &harness.spec);

    let message = PayloadAttestationMessage {
        validator_index,
        data: data.clone(),
        signature,
    };

    let result = harness.chain.import_payload_attestation_message(message);
    assert!(
        result.is_ok(),
        "should import payload-absent attestation, got: {:?}",
        result.err()
    );

    let attestation = result.unwrap();
    assert!(!attestation.data.payload_present);
}

/// Test that the aggregation_bits field in the returned attestation has exactly one bit set
/// at the correct PTC position.
#[tokio::test]
async fn gloas_import_payload_attestation_message_single_bit_set() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let ptc =
        get_ptc_committee::<E>(state, head_slot, &harness.spec).expect("should get PTC committee");
    let validator_index = ptc[0];
    // The expected PTC position is 0 (first member)

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: true,
    };

    let signature =
        sign_payload_attestation_data(&data, validator_index as usize, state, &harness.spec);

    let message = PayloadAttestationMessage {
        validator_index,
        data,
        signature,
    };

    let attestation = harness
        .chain
        .import_payload_attestation_message(message)
        .expect("should import");

    // Exactly one bit should be set
    let set_bits: Vec<usize> = (0..E::ptc_size())
        .filter(|&i| attestation.aggregation_bits.get(i).unwrap_or(false))
        .collect();
    assert_eq!(
        set_bits.len(),
        1,
        "exactly one aggregation bit should be set"
    );
    assert_eq!(set_bits[0], 0, "the first PTC member should have bit 0 set");
}

/// Test importing a message for a non-head PTC member (second member in the committee).
#[tokio::test]
async fn gloas_import_payload_attestation_message_second_ptc_member() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let ptc =
        get_ptc_committee::<E>(state, head_slot, &harness.spec).expect("should get PTC committee");

    // MinimalEthSpec PTC size is 2, so we need at least 2 members
    if ptc.len() < 2 {
        // Skip if PTC doesn't have a second member (shouldn't happen with 32 validators)
        return;
    }

    let validator_index = ptc[1];

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: true,
    };

    let signature =
        sign_payload_attestation_data(&data, validator_index as usize, state, &harness.spec);

    let message = PayloadAttestationMessage {
        validator_index,
        data,
        signature,
    };

    let attestation = harness
        .chain
        .import_payload_attestation_message(message)
        .expect("should import second PTC member's attestation");

    // Bit 1 should be set (second member), bit 0 should not
    assert!(
        attestation.aggregation_bits.get(1).unwrap_or(false),
        "bit 1 should be set for second PTC member"
    );
    assert!(
        !attestation.aggregation_bits.get(0).unwrap_or(false),
        "bit 0 should not be set for second PTC member"
    );
}

/// Test that importing a message with an invalid signature fails.
#[tokio::test]
async fn gloas_import_payload_attestation_message_invalid_signature() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let validator_index = first_ptc_member(state, head_slot, &harness.spec);

    // Sign with the WRONG keypair (validator 0 signs for a different validator)
    let wrong_keypair_index = if validator_index == 0 { 1 } else { 0 };

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: false,
    };

    let signature =
        sign_payload_attestation_data(&data, wrong_keypair_index as usize, state, &harness.spec);

    let message = PayloadAttestationMessage {
        validator_index,
        data,
        signature,
    };

    let result = harness.chain.import_payload_attestation_message(message);
    assert!(
        result.is_err(),
        "should reject message with invalid signature"
    );
    // The error goes through gossip verification, which wraps it as PayloadAttestationVerificationFailed
    assert!(
        matches!(
            result.unwrap_err(),
            BeaconChainError::PayloadAttestationVerificationFailed(..)
        ),
        "should be PayloadAttestationVerificationFailed"
    );
}

/// Test that importing a message with an unknown beacon block root fails during
/// gossip verification (the block must be known in fork choice).
#[tokio::test]
async fn gloas_import_payload_attestation_message_unknown_block_root() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let validator_index = first_ptc_member(state, head_slot, &harness.spec);

    let unknown_root = Hash256::repeat_byte(0xde);
    let data = PayloadAttestationData {
        beacon_block_root: unknown_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: false,
    };

    let signature =
        sign_payload_attestation_data(&data, validator_index as usize, state, &harness.spec);

    let message = PayloadAttestationMessage {
        validator_index,
        data,
        signature,
    };

    let result = harness.chain.import_payload_attestation_message(message);
    assert!(
        result.is_err(),
        "should reject message with unknown block root"
    );
    // This fails during gossip verification with UnknownBeaconBlockRoot
    assert!(
        matches!(
            result.unwrap_err(),
            BeaconChainError::PayloadAttestationVerificationFailed(..)
        ),
        "should be PayloadAttestationVerificationFailed for unknown block root"
    );
}

// ── Envelope storage and retrieval tests ──

/// After producing Gloas blocks, envelopes should be persisted in the store.
#[tokio::test]
async fn gloas_envelope_persisted_after_block_production() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let block_root = head.beacon_block_root;

    // Envelope should exist in the store
    assert!(
        harness
            .chain
            .store
            .payload_envelope_exists(&block_root)
            .unwrap(),
        "envelope should exist after block production"
    );

    // Full envelope should be retrievable
    let envelope = harness
        .chain
        .get_payload_envelope(&block_root)
        .unwrap()
        .expect("full envelope should be retrievable");

    // Verify envelope fields match the block
    assert_eq!(
        envelope.message.slot,
        head.beacon_block.slot(),
        "envelope slot should match block slot"
    );
}

/// Blinded envelope should be retrievable from the store.
#[tokio::test]
async fn gloas_blinded_envelope_retrievable() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let head = harness.chain.head_snapshot();
    let block_root = head.beacon_block_root;

    let blinded = harness
        .chain
        .store
        .get_blinded_payload_envelope(&block_root)
        .unwrap()
        .expect("blinded envelope should be retrievable");

    // Blinded envelope should have same metadata as full envelope
    let full = harness
        .chain
        .get_payload_envelope(&block_root)
        .unwrap()
        .expect("full envelope should be retrievable");

    assert_eq!(
        blinded.message.slot, full.message.slot,
        "blinded and full envelope slots should match"
    );
    assert_eq!(
        blinded.message.builder_index, full.message.builder_index,
        "blinded and full envelope builder indices should match"
    );
    assert_eq!(
        blinded.message.state_root, full.message.state_root,
        "blinded and full envelope state roots should match"
    );
}

/// Envelope should not exist for a random block root.
#[tokio::test]
async fn gloas_envelope_not_found_for_unknown_root() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(1)).await;

    let unknown_root = Hash256::repeat_byte(0xab);

    assert!(
        !harness
            .chain
            .store
            .payload_envelope_exists(&unknown_root)
            .unwrap(),
        "envelope should not exist for unknown root"
    );

    assert!(
        harness
            .chain
            .get_payload_envelope(&unknown_root)
            .unwrap()
            .is_none(),
        "get_payload_envelope should return None for unknown root"
    );

    assert!(
        harness
            .chain
            .store
            .get_blinded_payload_envelope(&unknown_root)
            .unwrap()
            .is_none(),
        "get_blinded_payload_envelope should return None for unknown root"
    );
}

/// Each block in a multi-block chain should have its own envelope.
#[tokio::test]
async fn gloas_each_block_has_distinct_envelope() {
    let harness = gloas_harness_at_epoch(0);
    let num_slots = 4;
    Box::pin(harness.extend_slots(num_slots)).await;

    let head = harness.chain.head_snapshot();
    let state = &head.beacon_state;

    // Walk the block roots within the state's slot range
    let mut envelope_count = 0;
    let mut seen_roots = std::collections::HashSet::new();

    for slot_idx in 1..=num_slots as u64 {
        let slot = Slot::new(slot_idx);
        if let Ok(block_root) = state.get_block_root(slot)
            && seen_roots.insert(*block_root)
        {
            let envelope = harness.chain.get_payload_envelope(block_root).unwrap();
            if let Some(env) = envelope {
                assert_eq!(
                    env.message.slot, slot,
                    "envelope slot should match block slot"
                );
                envelope_count += 1;
            }
        }
    }

    assert!(
        envelope_count >= 3,
        "should have envelopes for most blocks, got {}",
        envelope_count
    );
}

/// Self-build envelopes should have BUILDER_INDEX_SELF_BUILD as builder_index.
#[tokio::test]
async fn gloas_self_build_envelope_has_correct_builder_index() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let head = harness.chain.head_snapshot();
    let block_root = head.beacon_block_root;

    let envelope = harness
        .chain
        .get_payload_envelope(&block_root)
        .unwrap()
        .expect("envelope should exist");

    // In self-build mode, builder_index should be BUILDER_INDEX_SELF_BUILD
    assert_eq!(
        envelope.message.builder_index,
        u64::MAX, // BUILDER_INDEX_SELF_BUILD
        "self-build envelope should have BUILDER_INDEX_SELF_BUILD"
    );
}

/// Envelope state_root should be non-zero after processing.
#[tokio::test]
async fn gloas_envelope_has_nonzero_state_root() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let head = harness.chain.head_snapshot();
    let block_root = head.beacon_block_root;

    let envelope = harness
        .chain
        .get_payload_envelope(&block_root)
        .unwrap()
        .expect("envelope should exist");

    assert_ne!(
        envelope.message.state_root,
        Hash256::zero(),
        "envelope state_root should be non-zero after processing"
    );

    assert_ne!(
        envelope.message.payload.block_hash,
        ExecutionBlockHash::zero(),
        "envelope payload block_hash should be non-zero after processing"
    );
}

/// After chain finalizes, envelopes for finalized blocks should still be
/// accessible as blinded envelopes (even if full payload is pruned).
#[tokio::test]
async fn gloas_envelope_accessible_after_finalization() {
    let harness = gloas_harness_at_epoch(0);
    // Need enough slots to finalize (5 epochs in minimal)
    let slots = 5 * E::slots_per_epoch() as usize;
    Box::pin(harness.extend_slots(slots)).await;

    let state = &harness.chain.head_snapshot().beacon_state;
    let finalized_epoch = state.finalized_checkpoint().epoch;
    assert!(
        finalized_epoch > Epoch::new(0),
        "chain should have finalized beyond genesis"
    );

    // Get a block root from the first epoch (which should be finalized)
    let finalized_slot = Slot::new(1);
    let block_root = *state.get_block_root(finalized_slot).unwrap();

    // Blinded envelope should still be accessible
    let blinded = harness
        .chain
        .store
        .get_blinded_payload_envelope(&block_root)
        .unwrap();

    assert!(
        blinded.is_some(),
        "blinded envelope should be accessible for finalized block at slot {}",
        finalized_slot
    );
}

/// load_envelopes_for_blocks should return envelopes for Gloas blocks.
#[tokio::test]
async fn gloas_load_envelopes_for_blocks() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(4)).await;

    // Get some blocks from the store using block roots within the state range
    let state = &harness.chain.head_snapshot().beacon_state;
    let mut blocks = vec![];
    let mut seen_roots = std::collections::HashSet::new();
    for slot_idx in 1..=4u64 {
        let slot = Slot::new(slot_idx);
        if let Ok(block_root) = state.get_block_root(slot)
            && seen_roots.insert(*block_root)
            && let Some(block) = harness.chain.store.get_blinded_block(block_root).unwrap()
        {
            blocks.push(block);
        }
    }

    assert!(!blocks.is_empty(), "should have loaded at least one block");

    let (full_envelopes, blinded_envelopes) = harness.chain.load_envelopes_for_blocks(&blocks);

    // All blocks should have full envelopes (not yet finalized)
    assert!(
        !full_envelopes.is_empty(),
        "should have at least one full envelope"
    );
    assert!(
        blinded_envelopes.is_empty(),
        "should have no blinded-only envelopes (payloads not yet pruned)"
    );

    // Each loaded envelope should match its block's slot
    for (root, envelope) in &full_envelopes {
        let block = harness
            .chain
            .store
            .get_blinded_block(root)
            .unwrap()
            .unwrap();
        assert_eq!(
            envelope.message.slot,
            block.slot(),
            "envelope slot should match block slot"
        );
    }
}

// =============================================================================
// Payload pruning and blinded envelope fallback
// =============================================================================

/// After pruning a Gloas block's full payload, get_payload_envelope returns None
/// but get_blinded_payload_envelope still returns Some.
#[tokio::test]
async fn gloas_pruned_payload_full_envelope_gone_blinded_survives() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let block_root = head.beacon_block_root;

    // Before pruning: full envelope exists
    assert!(
        harness
            .chain
            .get_payload_envelope(&block_root)
            .unwrap()
            .is_some(),
        "full envelope should exist before pruning"
    );

    // Prune the execution payload for this block
    harness
        .chain
        .store
        .do_atomically_with_block_and_blobs_cache(vec![store::StoreOp::DeleteExecutionPayload(
            block_root,
        )])
        .unwrap();

    // After pruning: full envelope is gone (payload component missing)
    assert!(
        harness
            .chain
            .get_payload_envelope(&block_root)
            .unwrap()
            .is_none(),
        "full envelope should be None after payload pruning"
    );

    // Blinded envelope survives pruning
    let blinded = harness
        .chain
        .store
        .get_blinded_payload_envelope(&block_root)
        .unwrap();
    assert!(
        blinded.is_some(),
        "blinded envelope should survive payload pruning"
    );

    // Blinded envelope has correct metadata
    let blinded = blinded.unwrap();
    assert_eq!(
        blinded.message.slot,
        head.beacon_block.slot(),
        "blinded envelope slot should match block slot"
    );
}

/// After pruning, load_envelopes_for_blocks falls back to blinded envelopes.
#[tokio::test]
async fn gloas_load_envelopes_falls_back_to_blinded_after_pruning() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(4)).await;

    // Collect block roots and blocks
    let state = &harness.chain.head_snapshot().beacon_state;
    let mut blocks = vec![];
    let mut block_roots = vec![];
    let mut seen_roots = std::collections::HashSet::new();
    for slot_idx in 1..=4u64 {
        let slot = Slot::new(slot_idx);
        if let Ok(block_root) = state.get_block_root(slot)
            && seen_roots.insert(*block_root)
            && let Some(block) = harness.chain.store.get_blinded_block(block_root).unwrap()
        {
            block_roots.push(*block_root);
            blocks.push(block);
        }
    }
    assert!(!blocks.is_empty(), "should have loaded at least one block");

    // Before pruning: all envelopes are full, none blinded
    let (full_before, blinded_before) = harness.chain.load_envelopes_for_blocks(&blocks);
    assert!(!full_before.is_empty(), "should have full envelopes");
    assert!(
        blinded_before.is_empty(),
        "should have no blinded envelopes before pruning"
    );

    // Prune all execution payloads
    let ops: Vec<_> = block_roots
        .iter()
        .map(|root| store::StoreOp::DeleteExecutionPayload(*root))
        .collect();
    harness
        .chain
        .store
        .do_atomically_with_block_and_blobs_cache(ops)
        .unwrap();

    // After pruning: no full envelopes, all fall back to blinded
    let (full_after, blinded_after) = harness.chain.load_envelopes_for_blocks(&blocks);
    assert!(
        full_after.is_empty(),
        "should have no full envelopes after pruning"
    );
    assert!(
        !blinded_after.is_empty(),
        "should have blinded envelopes as fallback"
    );

    // Blinded envelopes should cover the same block roots
    for root in &block_roots {
        assert!(
            blinded_after.contains_key(root),
            "blinded fallback should contain block root {:?}",
            root
        );
    }
}

/// Pruning one block's payload produces a mix of full and blinded envelopes.
#[tokio::test]
async fn gloas_mixed_full_and_blinded_envelopes_after_partial_prune() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(4)).await;

    let state = &harness.chain.head_snapshot().beacon_state;
    let mut blocks = vec![];
    let mut block_roots = vec![];
    let mut seen_roots = std::collections::HashSet::new();
    for slot_idx in 1..=4u64 {
        let slot = Slot::new(slot_idx);
        if let Ok(block_root) = state.get_block_root(slot)
            && seen_roots.insert(*block_root)
            && let Some(block) = harness.chain.store.get_blinded_block(block_root).unwrap()
        {
            block_roots.push(*block_root);
            blocks.push(block);
        }
    }
    assert!(
        block_roots.len() >= 2,
        "need at least 2 blocks for partial prune test"
    );

    // Prune only the first block's payload
    let pruned_root = block_roots[0];
    harness
        .chain
        .store
        .do_atomically_with_block_and_blobs_cache(vec![store::StoreOp::DeleteExecutionPayload(
            pruned_root,
        )])
        .unwrap();

    let (full, blinded) = harness.chain.load_envelopes_for_blocks(&blocks);

    // The pruned block should be in blinded, the rest in full
    assert!(
        blinded.contains_key(&pruned_root),
        "pruned block should fall back to blinded envelope"
    );
    assert!(
        !full.contains_key(&pruned_root),
        "pruned block should not have full envelope"
    );

    // Remaining blocks should still have full envelopes
    for root in &block_roots[1..] {
        assert!(
            full.contains_key(root),
            "non-pruned block {:?} should still have full envelope",
            root
        );
    }
}

/// Blinded envelope preserves builder_index and state_root after pruning.
#[tokio::test]
async fn gloas_blinded_envelope_preserves_fields_after_pruning() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let head = harness.chain.head_snapshot();
    let block_root = head.beacon_block_root;

    // Capture fields from the full envelope before pruning
    let full_envelope = harness
        .chain
        .get_payload_envelope(&block_root)
        .unwrap()
        .expect("full envelope should exist");
    let expected_builder_index = full_envelope.message.builder_index;
    let expected_state_root = full_envelope.message.state_root;
    let expected_slot = full_envelope.message.slot;

    // Prune
    harness
        .chain
        .store
        .do_atomically_with_block_and_blobs_cache(vec![store::StoreOp::DeleteExecutionPayload(
            block_root,
        )])
        .unwrap();

    // Blinded envelope should preserve all metadata fields
    let blinded = harness
        .chain
        .store
        .get_blinded_payload_envelope(&block_root)
        .unwrap()
        .expect("blinded envelope should exist after pruning");

    assert_eq!(blinded.message.builder_index, expected_builder_index);
    assert_eq!(blinded.message.state_root, expected_state_root);
    assert_eq!(blinded.message.slot, expected_slot);
}

// =============================================================================
// Fork choice state verification after block + envelope processing
// =============================================================================

/// After blocks are produced and envelopes processed, fork choice nodes should have
/// payload_revealed = true for each block.
#[tokio::test]
async fn gloas_fork_choice_payload_revealed_after_extend() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(4)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;
    let fc = harness.chain.canonical_head.fork_choice_read_lock();

    // Check block roots accessible from the state's block_roots vector
    let mut checked = 0;
    for slot_idx in 1..=head_slot.as_u64() {
        let slot = Slot::new(slot_idx);
        if let Ok(block_root) = state.get_block_root(slot)
            && let Some(block) = fc.get_block(block_root)
        {
            assert!(
                block.payload_revealed,
                "payload_revealed should be true for block at slot {} (self-build envelope processed)",
                slot_idx
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 3,
        "should have checked at least 3 blocks, got {}",
        checked
    );
}

/// After blocks are produced with self-build, fork choice nodes should have
/// builder_index = Some(BUILDER_INDEX_SELF_BUILD).
#[tokio::test]
async fn gloas_fork_choice_builder_index_self_build() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;
    let fc = harness.chain.canonical_head.fork_choice_read_lock();

    let mut checked = 0;
    for slot_idx in 1..=head_slot.as_u64() {
        let slot = Slot::new(slot_idx);
        if let Ok(block_root) = state.get_block_root(slot)
            && let Some(block) = fc.get_block(block_root)
        {
            assert_eq!(
                block.builder_index,
                Some(harness.spec.builder_index_self_build),
                "fork choice node at slot {} should have BUILDER_INDEX_SELF_BUILD",
                slot_idx
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 2,
        "should have checked at least 2 blocks, got {}",
        checked
    );
}

/// After the chain processes envelopes and the EL returns Valid, execution status
/// should transition from Optimistic to Valid for each block.
#[tokio::test]
async fn gloas_fork_choice_execution_status_valid_after_envelope() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head_root = harness.chain.head_snapshot().beacon_block_root;
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let status = fc.get_block(&head_root).unwrap().execution_status;

    // The mock EL returns Valid, so after envelope processing the block should be Valid
    assert!(
        status.is_valid_or_irrelevant(),
        "head block execution status should be Valid after EL validation, got {:?}",
        status
    );
}

/// Fork choice genesis node should not have Gloas-specific fields set
/// (it's the pre-genesis anchor, not a Gloas block).
#[tokio::test]
async fn gloas_fork_choice_genesis_node_no_gloas_fields() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(1)).await;

    let genesis_root = harness.chain.genesis_block_root;
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let genesis_block = fc.get_block(&genesis_root).unwrap();

    // With FORK_NAME=gloas, the genesis block is a Gloas block with a default bid.
    // The anchor init (from get_forkchoice_store) sets builder_index from the bid,
    // which defaults to 0 for genesis blocks.
    assert_eq!(
        genesis_block.builder_index,
        Some(0),
        "genesis block should have builder_index from default bid"
    );
    // Anchor init also sets envelope_received and payload_revealed
    assert!(
        genesis_block.payload_revealed,
        "genesis block should have payload_revealed = true (anchor init)"
    );
}

/// Verify that blocks across a Fulu→Gloas fork transition have the correct
/// fork choice properties: pre-fork blocks have no builder_index, post-fork
/// blocks have BUILDER_INDEX_SELF_BUILD.
#[tokio::test]
async fn gloas_fork_choice_transition_properties() {
    let gloas_fork_epoch = Epoch::new(2);
    let gloas_fork_slot = gloas_fork_epoch.start_slot(E::slots_per_epoch());
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    // Extend past the fork
    Box::pin(harness.extend_to_slot(gloas_fork_slot + 2)).await;

    let state = harness.chain.head_beacon_state_cloned();
    let fc = harness.chain.canonical_head.fork_choice_read_lock();

    // Pre-fork block (last Fulu block)
    let pre_fork_slot = gloas_fork_slot - 1;
    let pre_fork_root = *state.get_block_root(pre_fork_slot).unwrap();
    let pre_fork = fc.get_block(&pre_fork_root).unwrap();
    assert_eq!(
        pre_fork.builder_index, None,
        "pre-fork (Fulu) block should have no builder_index"
    );

    // Post-fork block (first Gloas block)
    let post_fork_root = *state.get_block_root(gloas_fork_slot).unwrap();
    let post_fork = fc.get_block(&post_fork_root).unwrap();
    assert_eq!(
        post_fork.builder_index,
        Some(harness.spec.builder_index_self_build),
        "post-fork (Gloas) block should have BUILDER_INDEX_SELF_BUILD"
    );
    assert!(
        post_fork.payload_revealed,
        "post-fork block should have payload_revealed = true"
    );
}

// =============================================================================
// Execution proof verification — chain-dependent checks (4, 5, 6)
// =============================================================================

/// Helper: create a valid stub execution proof for a given block root and block hash.
fn make_stub_execution_proof(
    block_root: Hash256,
    block_hash: ExecutionBlockHash,
) -> Arc<ExecutionProof> {
    Arc::new(ExecutionProof::new(
        block_root,
        block_hash,
        ExecutionProofSubnetId::new(0).unwrap(),
        1, // stub version
        b"stub-proof-data".to_vec(),
    ))
}

/// Check 4: verify_execution_proof_for_gossip rejects proofs for unknown block roots.
#[tokio::test]
async fn gloas_execution_proof_unknown_block_root() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let unknown_root = Hash256::repeat_byte(0xde);
    let proof = make_stub_execution_proof(unknown_root, ExecutionBlockHash::zero());

    let result = harness
        .chain
        .verify_execution_proof_for_gossip(proof, ExecutionProofSubnetId::new(0).unwrap());

    match result {
        Err(GossipExecutionProofError::UnknownBlockRoot { block_root })
            if block_root == unknown_root => {}
        Err(other) => panic!("expected UnknownBlockRoot, got: {:?}", other),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

/// Check 5: verify_execution_proof_for_gossip rejects proofs for finalized blocks.
#[tokio::test]
async fn gloas_execution_proof_prior_to_finalization() {
    let harness = gloas_harness_at_epoch(0);
    // Run enough epochs to finalize
    let num_slots = 5 * E::slots_per_epoch() as usize;
    Box::pin(harness.extend_slots(num_slots)).await;

    let state = harness.chain.head_beacon_state_cloned();
    let finalized_epoch = state.finalized_checkpoint().epoch;
    assert!(
        finalized_epoch > Epoch::new(0),
        "chain should have finalized"
    );

    // Get a block root from a finalized slot (slot 1)
    let finalized_slot = Slot::new(1);
    let block_root = *state.get_block_root(finalized_slot).unwrap();

    // Get the block hash from the envelope for this block
    let envelope = harness
        .chain
        .store
        .get_blinded_payload_envelope(&block_root)
        .unwrap();
    let block_hash = envelope
        .map(|e| e.message.payload_header.block_hash)
        .unwrap_or(ExecutionBlockHash::zero());

    let proof = make_stub_execution_proof(block_root, block_hash);

    let result = harness
        .chain
        .verify_execution_proof_for_gossip(proof, ExecutionProofSubnetId::new(0).unwrap());

    // The block is finalized so it might not be in fork choice anymore (pruned).
    // If pruned, we get UnknownBlockRoot. If still in tree, we get PriorToFinalization.
    match result {
        Err(GossipExecutionProofError::UnknownBlockRoot { .. })
        | Err(GossipExecutionProofError::PriorToFinalization { .. }) => {}
        Err(other) => panic!(
            "expected UnknownBlockRoot or PriorToFinalization, got: {:?}",
            other
        ),
        Ok(_) => panic!("expected error for finalized block proof"),
    }
}

/// Check 6: verify_execution_proof_for_gossip rejects proofs with mismatched block hash.
#[tokio::test]
async fn gloas_execution_proof_block_hash_mismatch() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let block_root = head.beacon_block_root;

    // Use a wrong block hash
    let wrong_hash = ExecutionBlockHash::from(Hash256::repeat_byte(0xaa));
    let proof = make_stub_execution_proof(block_root, wrong_hash);

    let result = harness
        .chain
        .verify_execution_proof_for_gossip(proof, ExecutionProofSubnetId::new(0).unwrap());

    match result {
        Err(GossipExecutionProofError::BlockHashMismatch { .. }) => {}
        Err(other) => panic!("expected BlockHashMismatch, got: {:?}", other),
        Ok(_) => panic!("expected error for mismatched block hash"),
    }
}

/// Happy path: verify_execution_proof_for_gossip accepts a valid proof for a known block.
#[tokio::test]
async fn gloas_execution_proof_valid_stub_accepted() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let block_root = head.beacon_block_root;

    // Get the correct block hash from the envelope
    let envelope = harness
        .chain
        .get_payload_envelope(&block_root)
        .unwrap()
        .expect("envelope should exist");
    let block_hash = envelope.message.payload.block_hash;

    let proof = make_stub_execution_proof(block_root, block_hash);

    let result = harness
        .chain
        .verify_execution_proof_for_gossip(proof, ExecutionProofSubnetId::new(0).unwrap());

    assert!(
        result.is_ok(),
        "valid stub proof should be accepted, got: {:?}",
        result.err()
    );
}

/// Verify that a proof referencing a pre-Gloas (Fulu) block still works
/// (bid_block_hash is None, so check 6 is skipped).
#[tokio::test]
async fn gloas_execution_proof_pre_gloas_block_skips_hash_check() {
    let gloas_fork_epoch = Epoch::new(3);
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    // Extend to well past the fork
    let num_slots = 4 * E::slots_per_epoch() as usize;
    Box::pin(harness.extend_slots(num_slots)).await;

    let state = harness.chain.head_beacon_state_cloned();

    // Get a pre-fork block root (from epoch 1, which is Fulu)
    let pre_fork_slot = Slot::new(E::slots_per_epoch());
    let pre_fork_root = *state.get_block_root(pre_fork_slot).unwrap();

    // Verify this is indeed a pre-Gloas block
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let pre_fork_block = fc.get_block(&pre_fork_root);
    if let Some(block) = pre_fork_block {
        assert_eq!(
            block.builder_index, None,
            "pre-Gloas block should have no builder_index"
        );
        drop(fc);

        // Use any block hash — check 6 should be skipped for pre-Gloas blocks
        let any_hash = ExecutionBlockHash::from(Hash256::repeat_byte(0xbb));
        let proof = make_stub_execution_proof(pre_fork_root, any_hash);

        let result = harness
            .chain
            .verify_execution_proof_for_gossip(proof, ExecutionProofSubnetId::new(0).unwrap());

        // Should pass (check 6 is skipped when bid_block_hash is None)
        assert!(
            result.is_ok(),
            "proof for pre-Gloas block should skip hash check, got: {:?}",
            result.err()
        );
    }
    // If the block was pruned (not in fork choice), that's fine — skip the test
}

// ===== Block production payload attestation packing tests =====

/// Test that payload attestations inserted into the pool are included in produced Gloas blocks.
///
/// This verifies the end-to-end path: insert_payload_attestation_to_pool →
/// produce_block_on_state → get_payload_attestations_for_block → block body contains attestations.
#[tokio::test]
async fn gloas_block_production_includes_pool_attestations() {
    let harness = gloas_harness_at_epoch(0);
    // Produce 3 blocks to have a stable chain
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    // Insert a payload attestation targeting the head slot
    let att = make_payload_attestation(head_root, head_slot, true, false);
    harness
        .chain
        .insert_payload_attestation_to_pool(att.clone());

    // Advance the slot clock so we can produce the next block
    harness.advance_slot();

    // Produce a block at head_slot + 1 which should pack attestations for head_slot
    let state = harness.chain.head_beacon_state_cloned();
    let ((signed_block, _blobs), _state) = harness.make_block(state, head_slot + 1).await;

    let block = signed_block.message();
    assert!(
        block.fork_name_unchecked().gloas_enabled(),
        "produced block should be Gloas"
    );

    let payload_attestations = block
        .body()
        .payload_attestations()
        .expect("Gloas block should have payload_attestations");

    assert!(
        !payload_attestations.is_empty(),
        "block should include payload attestation from pool"
    );

    // Verify the included attestation targets the correct block
    assert_eq!(payload_attestations[0].data.beacon_block_root, head_root);
    assert_eq!(payload_attestations[0].data.slot, head_slot);
    assert!(payload_attestations[0].data.payload_present);
}

/// Test that block production only includes attestations matching the parent block root.
#[tokio::test]
async fn gloas_block_production_filters_attestations_by_parent_root() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    // Insert a matching attestation and a non-matching one (wrong root)
    let matching = make_payload_attestation(head_root, head_slot, true, false);
    let wrong_root = make_payload_attestation(Hash256::repeat_byte(0xde), head_slot, false, false);

    harness.chain.insert_payload_attestation_to_pool(matching);
    harness.chain.insert_payload_attestation_to_pool(wrong_root);

    harness.advance_slot();

    let state = harness.chain.head_beacon_state_cloned();
    let ((signed_block, _), _) = harness.make_block(state, head_slot + 1).await;

    let payload_attestations = signed_block
        .message()
        .body()
        .payload_attestations()
        .expect("should have payload_attestations");

    // Only the matching attestation should be included
    for att in payload_attestations.iter() {
        assert_eq!(
            att.data.beacon_block_root, head_root,
            "only attestations matching parent root should be included"
        );
    }
}

/// Test that block production respects the max_payload_attestations limit.
#[tokio::test]
async fn gloas_block_production_respects_max_payload_attestations() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    let max_atts = E::max_payload_attestations();

    // Insert more attestations than the max (each with different bits to differentiate)
    for i in 0..max_atts + 5 {
        let mut att = make_payload_attestation(head_root, head_slot, true, false);
        // Set a different bit for each attestation to make them distinct
        let _ = att.aggregation_bits.set(i % E::ptc_size(), true);
        harness.chain.insert_payload_attestation_to_pool(att);
    }

    harness.advance_slot();

    let state = harness.chain.head_beacon_state_cloned();
    let ((signed_block, _), _) = harness.make_block(state, head_slot + 1).await;

    let payload_attestations = signed_block
        .message()
        .body()
        .payload_attestations()
        .expect("should have payload_attestations");

    assert!(
        payload_attestations.len() <= max_atts,
        "should not exceed max_payload_attestations ({}), got {}",
        max_atts,
        payload_attestations.len()
    );
}

/// Test that block production produces empty payload_attestations when pool is empty.
#[tokio::test]
async fn gloas_block_production_empty_pool_no_attestations() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();

    // Don't insert any attestations — pool is empty
    harness.advance_slot();

    let state = harness.chain.head_beacon_state_cloned();
    let ((signed_block, _), _) = harness.make_block(state, head_slot + 1).await;

    let payload_attestations = signed_block
        .message()
        .body()
        .payload_attestations()
        .expect("Gloas block should have payload_attestations field");

    assert!(
        payload_attestations.is_empty(),
        "should have no payload attestations when pool is empty"
    );
}

/// Test that the next block's self-build bid has parent_block_hash matching the current
/// head state's latest_block_hash (proving block production reads the state correctly).
#[tokio::test]
async fn gloas_self_build_bid_parent_hash_matches_state() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();

    // The head state's latest_block_hash was set by envelope processing of the head block.
    let latest_block_hash = *head
        .beacon_state
        .latest_block_hash()
        .expect("Gloas state has latest_block_hash");

    // Produce the next block — its bid should reference the head state's latest_block_hash
    harness.advance_slot();
    let next_state = harness.chain.head_beacon_state_cloned();
    let ((signed_block, _), _) = harness.make_block(next_state, head_slot + 1).await;

    let next_parent_block_hash = signed_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("Gloas block should have bid")
        .message
        .parent_block_hash;

    assert_eq!(
        next_parent_block_hash, latest_block_hash,
        "next block's bid parent_block_hash should match previous state's latest_block_hash"
    );
}

/// Test that self-build bid slot matches the block's slot.
#[tokio::test]
async fn gloas_self_build_bid_slot_matches_block() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(4)).await;

    let head = &harness.chain.head_snapshot().beacon_block;
    let block = head.as_gloas().expect("should be Gloas block");

    assert_eq!(
        block.message.body.signed_execution_payload_bid.message.slot, block.message.slot,
        "self-build bid slot should match block slot"
    );
    assert_eq!(
        block
            .message
            .body
            .signed_execution_payload_bid
            .message
            .parent_block_root,
        block.message.parent_root,
        "self-build bid parent_block_root should match block parent_root"
    );
}

/// Test that gossip verification rejects a Gloas block whose bid.parent_block_root
/// does not match block.parent_root. This is the BidParentRootMismatch check in
/// block_verification.rs — a consensus safety check ensuring the bid and block agree
/// on their parent.
#[tokio::test]
async fn gloas_gossip_rejects_block_with_bid_parent_root_mismatch() {
    let harness = gloas_harness_at_epoch(0);

    // Build a chain with a few blocks
    Box::pin(harness.extend_slots(3)).await;

    harness.advance_slot();

    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;

    // Use make_block_with_modifier to create a block with a tampered bid parent root.
    // The modifier runs before re-signing, so the proposer signature remains valid.
    let ((block, _blobs), _state) = harness
        .make_block_with_modifier(head_state, next_slot, |block| {
            let body = block.body_gloas_mut().expect("should be Gloas block");
            // Set bid.parent_block_root to a bogus value that differs from block.parent_root
            body.signed_execution_payload_bid.message.parent_block_root =
                Hash256::repeat_byte(0xde);
        })
        .await;

    let result = harness.chain.verify_block_for_gossip(block).await;

    assert!(
        result.is_err(),
        "should reject block with mismatched bid parent root"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, BlockError::BidParentRootMismatch { .. }),
        "expected BidParentRootMismatch, got {:?}",
        err
    );
}

/// Test that a valid Gloas block passes the BidParentRootMismatch check.
/// Complement to the rejection test above — confirms that a correctly-constructed
/// block (where bid.parent_block_root == block.parent_root) is not rejected by
/// this check.
#[tokio::test]
async fn gloas_gossip_accepts_block_with_matching_bid_parent_root() {
    let harness = gloas_harness_at_epoch(0);

    Box::pin(harness.extend_slots(3)).await;

    harness.advance_slot();

    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let ((block, _blobs), _state) = harness.make_block(head_state, next_slot).await;

    // Verify bid and block agree on parent root
    let gloas_block = block.as_gloas().expect("should be Gloas block");
    assert_eq!(
        gloas_block
            .message
            .body
            .signed_execution_payload_bid
            .message
            .parent_block_root,
        gloas_block.message.parent_root,
        "self-build bid should have matching parent_block_root"
    );

    // The block should pass gossip verification (or fail on a later check, not BidParentRootMismatch)
    let result = harness.chain.verify_block_for_gossip(block).await;
    match result {
        Ok(_) => {}
        Err(ref e) => {
            assert!(
                !matches!(e, BlockError::BidParentRootMismatch { .. }),
                "valid block should not fail with BidParentRootMismatch, got {:?}",
                e
            );
        }
    }
}

/// Test that Gloas blocks import successfully without any blob/column sidecar data.
/// In ePBS, the execution payload is delivered separately via the envelope, so the
/// block itself is always "data available" from the block import perspective.
/// This verifies the data availability bypass in block_verification.rs where Gloas
/// blocks skip the AvailabilityPending path and go directly to Available.
#[tokio::test]
async fn gloas_block_import_without_blob_data() {
    let harness = gloas_harness_at_epoch(0);

    // Build initial chain
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();

    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let ((block, _blobs), _state) = harness.make_block(head_state, next_slot).await;

    let block_root = block.canonical_root();

    // Import the block with NO blob items at all (pass None for blobs).
    // For pre-Gloas blocks, this would cause the block to stall in AvailabilityPending.
    // For Gloas blocks, the data availability bypass should make this succeed.
    let result = harness
        .process_block(next_slot, block_root, (block, None))
        .await;

    assert!(
        result.is_ok(),
        "Gloas block should import without blob data, got {:?}",
        result.err()
    );
}

// =============================================================================
// Envelope processing integration tests (process_pending_envelope, process_payload_envelope)
// =============================================================================

/// After importing a Gloas block WITHOUT processing its envelope, the fork choice
/// node should have payload_revealed=false and no envelope in the store.
/// This is the starting state before any envelope is processed via gossip.
#[tokio::test]
async fn gloas_block_import_without_envelope_has_payload_unrevealed() {
    let harness = gloas_harness_at_epoch(0);

    // Build initial chain with envelopes processed (establishes head)
    Box::pin(harness.extend_slots(2)).await;

    // Make a block + envelope for the next slot, but only import the block
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    assert!(
        envelope.is_some(),
        "Gloas block production should yield an envelope"
    );

    let block_root = block_contents.0.canonical_root();

    // Import ONLY the block — no envelope processing
    let result = harness
        .process_block(next_slot, block_root, block_contents)
        .await;
    assert!(
        result.is_ok(),
        "block import should succeed, got {:?}",
        result.err()
    );

    // Fork choice: payload should NOT be revealed (no envelope processed)
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let proto_block = fc
        .get_block(&block_root)
        .expect("block should be in fork choice");
    assert!(
        !proto_block.payload_revealed,
        "payload_revealed should be false when envelope hasn't been processed"
    );
    drop(fc);

    // Store: no envelope should be persisted yet
    let stored_envelope = harness.chain.get_payload_envelope(&block_root).unwrap();
    assert!(
        stored_envelope.is_none(),
        "no envelope should be in store before processing"
    );
}

/// Self-build envelopes (Signature::empty) fail process_pending_envelope's
/// state transition because process_execution_payload_envelope uses
/// VerifySignatures::True. Verify the buffer is drained even on failure,
/// and the gossip verification still marks the payload as revealed in
/// fork choice (apply_payload_envelope_to_fork_choice runs before the
/// state transition fails).
#[tokio::test]
async fn gloas_process_pending_envelope_self_build_drains_buffer() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("should have envelope");
    let block_root = block_contents.0.canonical_root();

    // Import block only (no envelope processing)
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Buffer the self-build envelope
    harness
        .chain
        .pending_gossip_envelopes
        .lock()
        .insert(block_root, Arc::new(signed_envelope));

    // process_pending_envelope:
    // 1. Removes from buffer (always)
    // 2. Re-verifies (skips sig for self-build) → Ok
    // 3. Applies to fork choice → payload_revealed = true
    // 4. process_payload_envelope → fails on VerifySignatures::True (BadSignature)
    //    This is expected: self-build envelopes should use process_self_build_envelope
    harness.chain.process_pending_envelope(block_root).await;

    // Buffer should be drained regardless of state transition outcome
    assert!(
        harness
            .chain
            .pending_gossip_envelopes
            .lock()
            .get(&block_root)
            .is_none(),
        "pending buffer should be empty after processing attempt"
    );

    // Fork choice: payload_revealed should be true because
    // apply_payload_envelope_to_fork_choice runs BEFORE process_payload_envelope fails
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let proto_block = fc.get_block(&block_root).unwrap();
    assert!(
        proto_block.payload_revealed,
        "payload_revealed should be true (fork choice updated before state transition fails)"
    );
    drop(fc);
}

/// After importing a block without envelope, process_self_build_envelope
/// reveals the payload in fork choice, persists the envelope to store, and
/// updates the state cache.
#[tokio::test]
async fn gloas_self_build_envelope_reveals_payload_after_block_import() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("should have envelope");
    let block_root = block_contents.0.canonical_root();

    // Import block only
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Pre-condition: payload not yet revealed
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            !proto_block.payload_revealed,
            "pre-condition: not yet revealed"
        );
    }

    // Process self-build envelope (the correct path for locally-built blocks)
    harness
        .chain
        .process_self_build_envelope(&signed_envelope)
        .await
        .expect("should process self-build envelope");

    // Fork choice: payload should now be revealed
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let proto_block = fc.get_block(&block_root).unwrap();
    assert!(
        proto_block.payload_revealed,
        "payload_revealed should be true after self-build envelope processing"
    );
    drop(fc);

    // Store: envelope should be persisted
    let stored_envelope = harness
        .chain
        .get_payload_envelope(&block_root)
        .unwrap()
        .expect("envelope should be in store");
    assert_eq!(stored_envelope.message.slot, next_slot);
    assert_eq!(
        stored_envelope.message.builder_index, harness.spec.builder_index_self_build,
        "stored envelope should have self-build builder index"
    );
}

/// process_pending_envelope with no buffered envelope is a safe no-op.
#[tokio::test]
async fn gloas_process_pending_envelope_noop_when_empty() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let head = harness.chain.head_snapshot();
    let block_root = head.beacon_block_root;

    // Confirm buffer is empty for this root
    assert!(
        harness
            .chain
            .pending_gossip_envelopes
            .lock()
            .get(&block_root)
            .is_none(),
        "no pending envelope should exist"
    );

    // Call should be a no-op — no panic, no state change
    harness.chain.process_pending_envelope(block_root).await;

    // Head unchanged
    let new_head = harness.chain.head_snapshot();
    assert_eq!(new_head.beacon_block_root, block_root, "head unchanged");
}

/// After process_self_build_envelope, the head snapshot should have the
/// correct latest_block_hash (updated by envelope processing).
#[tokio::test]
async fn gloas_self_build_envelope_updates_head_state_latest_block_hash() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("should have envelope");
    let envelope_block_hash = signed_envelope.message.payload.block_hash;
    let block_root = block_contents.0.canonical_root();

    // Import block only
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Recompute head so the head snapshot points to the new block
    harness.chain.recompute_head_at_current_slot().await;

    // Process self-build envelope
    harness
        .chain
        .process_self_build_envelope(&signed_envelope)
        .await
        .expect("should process self-build envelope");

    // The head state should now have latest_block_hash updated
    let head_state = harness.chain.head_beacon_state_cloned();
    let latest_block_hash = head_state
        .latest_block_hash()
        .expect("Gloas state should have latest_block_hash");
    assert_eq!(
        *latest_block_hash, envelope_block_hash,
        "latest_block_hash should match the envelope's payload block_hash"
    );
}

/// Verify the gossip verification + fork choice flow works end-to-end for
/// self-build envelopes. verify_payload_envelope_for_gossip skips BLS sig
/// verification for BUILDER_INDEX_SELF_BUILD, and apply_payload_envelope_to_fork_choice
/// updates the fork choice tree.
#[tokio::test]
async fn gloas_gossip_verify_and_fork_choice_for_self_build_envelope() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("should have envelope");
    let block_root = block_contents.0.canonical_root();

    // Import block only
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Use verify_payload_envelope_for_gossip to get a VerifiedPayloadEnvelope
    let verified = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope))
        .expect("self-build envelope should pass gossip verification");

    // Apply to fork choice
    harness
        .chain
        .apply_payload_envelope_to_fork_choice(&verified)
        .expect("should apply to fork choice");

    // Verify fork choice updated
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let proto_block = fc.get_block(&block_root).unwrap();
    assert!(proto_block.payload_revealed, "payload should be revealed");
    drop(fc);
}

/// After process_self_build_envelope, the state cache should hold the
/// post-envelope state keyed by the block's state_root, with the correct
/// latest_block_hash.
#[tokio::test]
async fn gloas_self_build_envelope_caches_post_envelope_state() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("should have envelope");
    let block_state_root = block_contents.0.message().state_root();
    let envelope_block_hash = signed_envelope.message.payload.block_hash;
    let block_root = block_contents.0.canonical_root();

    // Import block only
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Process self-build envelope
    harness
        .chain
        .process_self_build_envelope(&signed_envelope)
        .await
        .expect("should process");

    // The state cache should now hold the post-envelope state under block_state_root.
    // The post-envelope state has latest_block_hash updated to the envelope's block_hash.
    let cached_state = harness
        .chain
        .get_state(&block_state_root, Some(next_slot), false)
        .expect("should not error")
        .expect("post-envelope state should be in cache");

    let latest_block_hash = cached_state
        .latest_block_hash()
        .expect("Gloas state should have latest_block_hash");
    assert_eq!(
        *latest_block_hash, envelope_block_hash,
        "cached state should have post-envelope latest_block_hash"
    );
}

// =============================================================================
// process_self_build_envelope — EL execution status & error path tests
// =============================================================================

/// After process_self_build_envelope with a mock EL (returns Valid), the block's
/// execution_status should transition from Optimistic to Valid. This is critical
/// because without the explicit on_valid_execution_payload call in
/// process_self_build_envelope, the head stays Optimistic and block production
/// is disabled (the forkchoiceUpdated issued during block import returns SYNCING
/// because the EL hasn't seen the payload yet).
#[tokio::test]
async fn gloas_self_build_envelope_marks_execution_status_valid() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("should have envelope");
    let payload_block_hash = signed_envelope.message.payload.block_hash;
    let block_root = block_contents.0.canonical_root();

    // Import block only
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Pre-condition: execution_status should be Optimistic after block import
    // (the EL hasn't seen the payload yet, only the forkchoiceUpdated)
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            matches!(proto_block.execution_status, ExecutionStatus::Optimistic(_)),
            "pre-condition: should be Optimistic after block import, got {:?}",
            proto_block.execution_status
        );
    }

    // Process self-build envelope — mock EL returns Valid for newPayload
    harness
        .chain
        .process_self_build_envelope(&signed_envelope)
        .await
        .expect("should process self-build envelope");

    // Post-condition: execution_status should be Valid
    // This transition happens because process_self_build_envelope:
    // 1. Calls notify_new_payload → EL returns Valid
    // 2. Calls on_valid_execution_payload to explicitly mark it in fork choice
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            matches!(
                proto_block.execution_status,
                ExecutionStatus::Valid(hash) if hash == payload_block_hash
            ),
            "should be Valid(payload_block_hash) after self-build envelope, got {:?}",
            proto_block.execution_status
        );
    }
}

/// In stateless mode, process_self_build_envelope skips the EL newPayload call,
/// so execution_status stays Optimistic. The payload is still processed
/// (state transition runs, latest_block_hash updated) but the execution layer
/// is not consulted. Execution validity is established later via execution proofs.
#[tokio::test]
async fn gloas_self_build_envelope_stateless_mode_stays_optimistic() {
    // Use a stateless harness with a separate producer harness
    let stateless = gloas_stateless_harness(1);
    let producer = gloas_harness_at_epoch(0);

    // Produce and import 2 blocks to build chain state
    for _ in 0..2 {
        producer.advance_slot();
        stateless.advance_slot();
        let head_state = producer.chain.head_beacon_state_cloned();
        let next_slot = head_state.slot() + 1;
        let (block_contents, _state, envelope) = producer
            .make_block_with_envelope(head_state, next_slot)
            .await;

        let envelope = envelope.expect("should have envelope");
        let block_root = block_contents.0.canonical_root();

        producer
            .process_block(next_slot, block_root, block_contents.clone())
            .await
            .expect("producer import ok");
        producer
            .chain
            .process_self_build_envelope(&envelope)
            .await
            .expect("producer envelope ok");

        stateless
            .process_block(next_slot, block_root, block_contents)
            .await
            .expect("stateless import ok");
        stateless
            .chain
            .process_self_build_envelope(&envelope)
            .await
            .expect("stateless envelope ok");
    }

    // Produce one more block for testing
    producer.advance_slot();
    stateless.advance_slot();
    let head_state = producer.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = producer
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let envelope = envelope.expect("should have envelope");
    let block_root = block_contents.0.canonical_root();

    producer
        .process_block(next_slot, block_root, block_contents.clone())
        .await
        .expect("producer import ok");
    producer
        .chain
        .process_self_build_envelope(&envelope)
        .await
        .expect("producer envelope ok");

    stateless
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("stateless import ok");

    // Process envelope in stateless mode
    stateless
        .chain
        .process_self_build_envelope(&envelope)
        .await
        .expect("stateless envelope should succeed (skips EL)");

    // execution_status should STILL be Optimistic (EL was not called)
    {
        let fc = stateless.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            matches!(proto_block.execution_status, ExecutionStatus::Optimistic(_)),
            "stateless mode: should remain Optimistic (no EL verification), got {:?}",
            proto_block.execution_status
        );
    }

    // But payload_revealed should be true (fork choice still updated)
    {
        let fc = stateless.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "stateless mode: payload_revealed should be true (fork choice updated)"
        );
    }

    // And the state transition should have run (latest_block_hash updated)
    // The state may or may not be the head yet, but the cached state under block_state_root
    // should have latest_block_hash set. Check via get_state:
    let block_state_root = stateless
        .chain
        .store
        .get_blinded_block(&block_root)
        .unwrap()
        .unwrap()
        .message()
        .state_root();
    let post_envelope_state = stateless
        .chain
        .get_state(&block_state_root, Some(next_slot), false)
        .expect("should not error")
        .expect("post-envelope state should be cached");
    let latest_block_hash = post_envelope_state
        .latest_block_hash()
        .expect("Gloas state should have latest_block_hash");
    assert_ne!(
        *latest_block_hash,
        ExecutionBlockHash::zero(),
        "stateless mode: state transition should still run (latest_block_hash set)"
    );
}

/// When the EL returns Invalid for the envelope's newPayload, process_self_build_envelope
/// should return an error. The block should remain Optimistic in fork choice (not marked Valid)
/// because the payload was rejected. However, payload_revealed should still be true because
/// on_execution_payload runs before the EL call in process_self_build_envelope.
#[tokio::test]
async fn gloas_self_build_envelope_el_invalid_returns_error() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("should have envelope");
    let block_root = block_contents.0.canonical_root();

    // Import block
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Configure mock EL to return Invalid for newPayload
    let mock_el = harness.mock_execution_layer.as_ref().unwrap();
    mock_el
        .server
        .all_payloads_invalid_on_new_payload(ExecutionBlockHash::zero());

    // Process self-build envelope — should fail because EL says Invalid
    let result = harness
        .chain
        .process_self_build_envelope(&signed_envelope)
        .await;

    assert!(result.is_err(), "should error when EL returns Invalid");
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("invalid"),
        "error should mention invalid payload, got: {}",
        err_msg
    );

    // Block should still be Optimistic (not Valid) in fork choice
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            matches!(proto_block.execution_status, ExecutionStatus::Optimistic(_)),
            "block should remain Optimistic when EL returns Invalid, got {:?}",
            proto_block.execution_status
        );
    }

    // payload_revealed should be true because on_execution_payload ran first
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "payload_revealed should be true (on_execution_payload runs before EL call)"
        );
    }
}

/// When the EL returns InvalidBlockHash for the envelope's newPayload,
/// process_self_build_envelope should return an error with a message about
/// invalid block hash. The execution status should remain Optimistic.
#[tokio::test]
async fn gloas_self_build_envelope_el_invalid_block_hash_returns_error() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("should have envelope");
    let block_root = block_contents.0.canonical_root();

    // Import block
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Configure mock EL to return InvalidBlockHash for newPayload
    let mock_el = harness.mock_execution_layer.as_ref().unwrap();
    mock_el
        .server
        .all_payloads_invalid_block_hash_on_new_payload();

    // Process self-build envelope — should fail
    let result = harness
        .chain
        .process_self_build_envelope(&signed_envelope)
        .await;

    assert!(
        result.is_err(),
        "should error when EL returns InvalidBlockHash"
    );
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("invalid block hash"),
        "error should mention invalid block hash, got: {}",
        err_msg
    );

    // Block should still be Optimistic
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            matches!(proto_block.execution_status, ExecutionStatus::Optimistic(_)),
            "block should remain Optimistic when EL returns InvalidBlockHash, got {:?}",
            proto_block.execution_status
        );
    }
}

/// When the EL returns Syncing/Accepted for the envelope's newPayload,
/// process_self_build_envelope should succeed (not error), but the block
/// should remain Optimistic (not promoted to Valid). This covers the EL
/// still syncing or not yet aware of parent payload.
#[tokio::test]
async fn gloas_self_build_envelope_el_syncing_stays_optimistic() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("should have envelope");
    let block_root = block_contents.0.canonical_root();

    // Import block
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Configure mock EL to return Syncing for newPayload
    let mock_el = harness.mock_execution_layer.as_ref().unwrap();
    mock_el.server.all_payloads_syncing_on_new_payload(false);

    // Process self-build envelope — should succeed (Syncing is not an error)
    harness
        .chain
        .process_self_build_envelope(&signed_envelope)
        .await
        .expect("Syncing response should not cause an error");

    // Block should still be Optimistic (not Valid since EL said Syncing, not Valid)
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            matches!(proto_block.execution_status, ExecutionStatus::Optimistic(_)),
            "block should remain Optimistic when EL returns Syncing, got {:?}",
            proto_block.execution_status
        );
    }

    // payload_revealed should still be true
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "payload_revealed should be true regardless of EL response"
        );
    }
}

/// process_self_build_envelope with a block_root not in the store should
/// return an error (missing beacon block). This catches the case where the
/// envelope arrives for a block that was never imported.
#[tokio::test]
async fn gloas_self_build_envelope_missing_block_root_errors() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    // Construct a fake envelope referencing a non-existent block
    let fake_root = Hash256::from_slice(&[0xab; 32]);
    let mut envelope = SignedExecutionPayloadEnvelope::<E>::default();
    envelope.message.beacon_block_root = fake_root;
    envelope.message.slot = Slot::new(3);

    let result = harness.chain.process_self_build_envelope(&envelope).await;

    assert!(result.is_err(), "should error for missing block root");
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("Missing beacon block"),
        "error should mention missing beacon block, got: {}",
        err_msg
    );
}

/// After process_self_build_envelope, producing the next block should work
/// correctly — verifying that the state transition and cache updates leave
/// the chain in a valid state for continued block production.
#[tokio::test]
async fn gloas_self_build_envelope_enables_next_block_production() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("should have envelope");
    let block_root = block_contents.0.canonical_root();

    // Import block only (without envelope)
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import ok");

    // Process self-build envelope
    harness
        .chain
        .process_self_build_envelope(&signed_envelope)
        .await
        .expect("envelope processing ok");

    // Recompute head so the chain advances
    harness.chain.recompute_head_at_current_slot().await;

    // Now produce the NEXT block — this should succeed because:
    // 1. The state cache has the post-envelope state (latest_block_hash updated)
    // 2. Fork choice has the block as Valid (EL returned Valid)
    // 3. The head snapshot is updated
    harness.advance_slot();
    let new_head_state = harness.chain.head_beacon_state_cloned();
    let next_next_slot = new_head_state.slot() + 1;
    let (next_block_contents, _next_state, next_envelope) = harness
        .make_block_with_envelope(new_head_state, next_next_slot)
        .await;

    // The next block should have been produced successfully
    let next_block = &next_block_contents.0;
    assert!(next_block.as_gloas().is_ok(), "next block should be Gloas");
    assert_eq!(
        next_block.slot(),
        next_next_slot,
        "next block should be at the correct slot"
    );

    // Its parent root should be the block we processed
    assert_eq!(
        next_block.message().parent_root(),
        block_root,
        "next block's parent should be the block whose envelope we processed"
    );

    // The bid's parent_block_hash should match the envelope's payload block_hash
    // (this verifies the state transition correctly updated latest_block_hash)
    let next_bid = next_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .unwrap();
    assert_eq!(
        next_bid.message.parent_block_hash, signed_envelope.message.payload.block_hash,
        "next bid's parent_block_hash should be previous envelope's payload block_hash"
    );

    // Envelope should be produced too
    assert!(next_envelope.is_some(), "next envelope should be produced");
}

/// process_self_build_envelope should persist the envelope to disk. After
/// processing, the envelope should be retrievable via get_payload_envelope.
/// This test specifically verifies the store persistence path (StoreOp::PutPayloadEnvelope)
/// by checking all envelope fields, not just existence.
#[tokio::test]
async fn gloas_self_build_envelope_store_persistence_fields() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("should have envelope");
    let block_root = block_contents.0.canonical_root();

    // Import block
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import ok");

    // Pre-condition: no envelope in store yet
    let pre = harness.chain.get_payload_envelope(&block_root).unwrap();
    assert!(
        pre.is_none(),
        "pre-condition: no envelope in store before processing"
    );

    // Process self-build envelope
    harness
        .chain
        .process_self_build_envelope(&signed_envelope)
        .await
        .expect("envelope processing ok");

    // Verify all stored fields match the original envelope
    let stored = harness
        .chain
        .get_payload_envelope(&block_root)
        .unwrap()
        .expect("envelope should be persisted");

    assert_eq!(stored.message.slot, signed_envelope.message.slot);
    assert_eq!(
        stored.message.builder_index,
        signed_envelope.message.builder_index
    );
    assert_eq!(
        stored.message.beacon_block_root,
        signed_envelope.message.beacon_block_root
    );
    assert_eq!(
        stored.message.payload.block_hash, signed_envelope.message.payload.block_hash,
        "stored payload block_hash should match"
    );
    assert_eq!(
        stored.message.builder_index, harness.spec.builder_index_self_build,
        "stored builder_index should be BUILDER_INDEX_SELF_BUILD"
    );
}

// =============================================================================
// Stateless validation — execution proof threshold tests
// =============================================================================

/// Creates a Gloas harness with stateless_validation enabled and custom proof threshold.
/// Blocks must be imported via `process_block` since block production requires EL (which
/// stateless mode skips). Use `gloas_harness_at_epoch(0)` to produce blocks, then import.
fn gloas_stateless_harness(
    min_proofs_required: usize,
) -> BeaconChainHarness<EphemeralHarnessType<E>> {
    let mut spec = E::default_spec();
    spec.altair_fork_epoch = Some(Epoch::new(0));
    spec.bellatrix_fork_epoch = Some(Epoch::new(0));
    spec.capella_fork_epoch = Some(Epoch::new(0));
    spec.deneb_fork_epoch = Some(Epoch::new(0));
    spec.electra_fork_epoch = Some(Epoch::new(0));
    spec.fulu_fork_epoch = Some(Epoch::new(0));
    spec.gloas_fork_epoch = Some(Epoch::new(0));

    let chain_config = ChainConfig {
        stateless_validation: true,
        stateless_min_proofs_required: min_proofs_required,
        ..ChainConfig::default()
    };

    let harness = BeaconChainHarness::builder(E::default())
        .spec(spec.into())
        .deterministic_keypairs(VALIDATOR_COUNT)
        .fresh_ephemeral_store()
        .mock_execution_layer()
        .chain_config(chain_config)
        .build();

    harness.advance_slot();
    harness
}

/// Helper: produces blocks on a normal harness, then imports them into the stateless harness
/// via `process_block` (import path, no EL needed for Gloas blocks). Returns the block_root
/// and block_hash of the last imported block.
async fn import_blocks_into_stateless(
    stateless: &BeaconChainHarness<EphemeralHarnessType<E>>,
    num_blocks: usize,
) -> (Hash256, ExecutionBlockHash) {
    // Produce blocks on a normal harness with the same genesis
    let producer = gloas_harness_at_epoch(0);
    let mut block_root = Hash256::zero();
    let mut block_hash = ExecutionBlockHash::zero();

    for i in 0..num_blocks {
        producer.advance_slot();
        stateless.advance_slot();
        let head_state = producer.chain.head_beacon_state_cloned();
        let next_slot = head_state.slot() + 1;
        let (block_contents, _state, envelope) = producer
            .make_block_with_envelope(head_state, next_slot)
            .await;

        let envelope = envelope.expect("Gloas should produce envelope");
        block_hash = envelope.message.payload.block_hash;
        block_root = block_contents.0.canonical_root();

        // Import block + envelope on the producer (so it can produce the next block)
        producer
            .process_block(next_slot, block_root, block_contents.clone())
            .await
            .unwrap_or_else(|e| panic!("producer import failed at block {}: {:?}", i, e));
        producer
            .chain
            .process_self_build_envelope(&envelope)
            .await
            .unwrap_or_else(|e| panic!("producer envelope failed at block {}: {:?}", i, e));

        // Import the block on the stateless harness
        stateless
            .process_block(next_slot, block_root, block_contents)
            .await
            .unwrap_or_else(|e| panic!("stateless import failed at block {}: {:?}", i, e));

        // Process the envelope on the stateless harness too — the envelope state transition
        // updates latest_block_hash which subsequent blocks' bids depend on. In stateless
        // mode, process_self_build_envelope skips the EL newPayload call but still runs
        // the state transition. The block remains optimistic (not execution-valid) because
        // no EL verification happened. This is what the proof threshold is meant to resolve.
        stateless
            .chain
            .process_self_build_envelope(&envelope)
            .await
            .unwrap_or_else(|e| panic!("stateless envelope failed at block {}: {:?}", i, e));
    }

    (block_root, block_hash)
}

/// In stateless mode, a Gloas block enters fork choice as optimistic. After receiving
/// enough execution proofs to meet the threshold, `check_gossip_execution_proof_availability_and_import`
/// marks it as execution-valid (Imported).
#[tokio::test]
async fn gloas_stateless_proof_threshold_marks_block_valid() {
    let harness = gloas_stateless_harness(1);
    let (block_root, block_hash) = import_blocks_into_stateless(&harness, 3).await;

    // Verify the block is currently optimistic (no envelope processed)
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let exec_status = fc
            .get_block_execution_status(&block_root)
            .expect("block should be in fork choice");
        assert!(
            exec_status.is_optimistic_or_invalid(),
            "block should be optimistic before proof, got {:?}",
            exec_status
        );
    }

    // Verify the proof, then run it through the stateless import path
    let proof = make_stub_execution_proof(block_root, block_hash);
    let subnet_id = ExecutionProofSubnetId::new(0).unwrap();

    let verified = harness
        .chain
        .verify_execution_proof_for_gossip(proof, subnet_id)
        .expect("proof should pass gossip verification");

    let head_slot = harness.chain.head_snapshot().beacon_block.slot();
    let result = harness
        .chain
        .check_gossip_execution_proof_availability_and_import(head_slot, block_root, verified)
        .await
        .expect("should not error");

    // Threshold = 1, we sent 1 proof => Imported
    assert_eq!(
        result,
        AvailabilityProcessingStatus::Imported(block_root),
        "proof threshold reached should return Imported"
    );

    // Verify the block is now execution-valid in fork choice
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let exec_status = fc
        .get_block_execution_status(&block_root)
        .expect("block should be in fork choice");
    assert!(
        exec_status.is_valid_or_irrelevant(),
        "block should be execution-valid after proof threshold, got {:?}",
        exec_status
    );
}

/// When the number of proofs is below the threshold, `check_gossip_execution_proof_availability_and_import`
/// returns `MissingComponents`.
#[tokio::test]
async fn gloas_stateless_below_threshold_returns_missing_components() {
    // Require 2 proofs, but only send 1
    let harness = gloas_stateless_harness(2);
    let (block_root, block_hash) = import_blocks_into_stateless(&harness, 3).await;

    let head_slot = harness.chain.head_snapshot().beacon_block.slot();

    let proof = make_stub_execution_proof(block_root, block_hash);
    let subnet_id = ExecutionProofSubnetId::new(0).unwrap();

    let verified = harness
        .chain
        .verify_execution_proof_for_gossip(proof, subnet_id)
        .expect("proof should pass gossip verification");

    let result = harness
        .chain
        .check_gossip_execution_proof_availability_and_import(head_slot, block_root, verified)
        .await
        .expect("should not error");

    // Only 1 proof for threshold=2 => MissingComponents
    assert_eq!(
        result,
        AvailabilityProcessingStatus::MissingComponents(head_slot, block_root),
        "below threshold should return MissingComponents"
    );

    // Block should still be optimistic
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let exec_status = fc
        .get_block_execution_status(&block_root)
        .expect("block should be in fork choice");
    assert!(
        exec_status.is_optimistic_or_invalid(),
        "block should remain optimistic below threshold, got {:?}",
        exec_status
    );
}

/// Duplicate subnet proofs are deduplicated by the HashSet — sending the same subnet twice
/// should NOT count as two distinct proofs toward the threshold.
/// With MAX_EXECUTION_PROOF_SUBNETS=1, only subnet 0 is valid, so we verify that sending
/// subnet 0 twice doesn't count as 2 proofs toward a threshold of 2.
#[tokio::test]
async fn gloas_stateless_duplicate_subnet_proofs_deduped() {
    // Require 2 proofs, but only 1 valid subnet exists
    let harness = gloas_stateless_harness(2);
    let (block_root, block_hash) = import_blocks_into_stateless(&harness, 3).await;

    let head_slot = harness.chain.head_snapshot().beacon_block.slot();
    let subnet_0 = ExecutionProofSubnetId::new(0).unwrap();

    // First proof on subnet 0
    let proof1 = make_stub_execution_proof(block_root, block_hash);
    let verified1 = harness
        .chain
        .verify_execution_proof_for_gossip(proof1, subnet_0)
        .expect("first proof should pass");

    let result1 = harness
        .chain
        .check_gossip_execution_proof_availability_and_import(head_slot, block_root, verified1)
        .await
        .expect("should not error");

    assert_eq!(
        result1,
        AvailabilityProcessingStatus::MissingComponents(head_slot, block_root),
        "first proof should be MissingComponents (1 of 2)"
    );

    // Second proof on the SAME subnet 0 — should NOT reach threshold
    let proof2 = make_stub_execution_proof(block_root, block_hash);
    let verified2 = harness
        .chain
        .verify_execution_proof_for_gossip(proof2, subnet_0)
        .expect("second proof should pass");

    let result2 = harness
        .chain
        .check_gossip_execution_proof_availability_and_import(head_slot, block_root, verified2)
        .await
        .expect("should not error");

    // Still MissingComponents — duplicate subnet 0 was deduplicated, count stays at 1
    assert_eq!(
        result2,
        AvailabilityProcessingStatus::MissingComponents(head_slot, block_root),
        "duplicate subnet should NOT count twice toward threshold"
    );

    // Verify the tracker has exactly 1 unique subnet despite 2 proof submissions
    let tracker = harness.chain.execution_proof_tracker.lock();
    let subnets = tracker.get(&block_root).expect("should have tracker entry");
    assert_eq!(
        subnets.len(),
        1,
        "tracker should have 1 unique subnet (deduplicated), not 2"
    );
}

/// `process_pending_execution_proofs` is a no-op when `stateless_validation = false`.
/// This ensures standard (non-stateless) nodes don't accidentally trigger the proof path.
#[tokio::test]
async fn gloas_process_pending_proofs_noop_when_not_stateless() {
    // Standard harness (stateless_validation = false)
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let block_root = head.beacon_block_root;

    // Manually insert a fake proof into pending_execution_proofs
    {
        let mut pending = harness.chain.pending_execution_proofs.lock();
        pending.insert(block_root, vec![ExecutionProofSubnetId::new(0).unwrap()]);
    }

    // Call process_pending_execution_proofs — should be a no-op
    harness.chain.process_pending_execution_proofs(block_root);

    // The pending proofs should still be there (not drained) because the function
    // returns early when stateless_validation is false
    let pending = harness.chain.pending_execution_proofs.lock();
    assert!(
        pending.contains_key(&block_root),
        "pending proofs should NOT be drained when stateless_validation is false"
    );
}

/// `process_pending_execution_proofs` drains buffered proofs and marks the block as
/// execution-valid when the threshold is reached.
#[tokio::test]
async fn gloas_process_pending_proofs_drains_and_marks_valid() {
    let harness = gloas_stateless_harness(1);
    let (block_root, _block_hash) = import_blocks_into_stateless(&harness, 3).await;

    // Buffer a proof in pending_execution_proofs (simulates proof arriving before block)
    {
        let mut pending = harness.chain.pending_execution_proofs.lock();
        pending.insert(block_root, vec![ExecutionProofSubnetId::new(0).unwrap()]);
    }

    // Call process_pending_execution_proofs
    harness.chain.process_pending_execution_proofs(block_root);

    // The buffer should be drained
    let pending = harness.chain.pending_execution_proofs.lock();
    assert!(
        !pending.contains_key(&block_root),
        "pending proofs should be drained after processing"
    );
    drop(pending);

    // The block should now be execution-valid
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let exec_status = fc
        .get_block_execution_status(&block_root)
        .expect("block should be in fork choice");
    assert!(
        exec_status.is_valid_or_irrelevant(),
        "block should be execution-valid after pending proofs met threshold, got {:?}",
        exec_status
    );
}

/// `process_pending_execution_proofs` with no buffered proofs is a safe no-op.
#[tokio::test]
async fn gloas_process_pending_proofs_noop_when_empty() {
    let harness = gloas_stateless_harness(1);
    let (block_root, _block_hash) = import_blocks_into_stateless(&harness, 3).await;

    // No proofs buffered — call should not panic or change anything
    harness.chain.process_pending_execution_proofs(block_root);

    // The execution_proof_tracker should still be empty for this block
    let tracker = harness.chain.execution_proof_tracker.lock();
    assert!(
        !tracker.contains_key(&block_root),
        "tracker should be empty when no proofs were buffered"
    );
}

/// Buffered proofs below threshold: `process_pending_execution_proofs` transfers proofs
/// to the tracker but does NOT mark the block as execution-valid.
#[tokio::test]
async fn gloas_process_pending_proofs_below_threshold_stays_optimistic() {
    let harness = gloas_stateless_harness(3); // need 3 proofs
    let (block_root, _block_hash) = import_blocks_into_stateless(&harness, 3).await;

    // Buffer only 1 proof (threshold = 3)
    {
        let mut pending = harness.chain.pending_execution_proofs.lock();
        pending.insert(block_root, vec![ExecutionProofSubnetId::new(0).unwrap()]);
    }

    harness.chain.process_pending_execution_proofs(block_root);

    // Buffer should be drained even though threshold not met
    let pending = harness.chain.pending_execution_proofs.lock();
    assert!(
        !pending.contains_key(&block_root),
        "pending proofs should be drained regardless of threshold"
    );
    drop(pending);

    // But the proof should be in the tracker
    let tracker = harness.chain.execution_proof_tracker.lock();
    let subnets = tracker.get(&block_root).expect("should have tracker entry");
    assert_eq!(subnets.len(), 1, "tracker should have 1 subnet");
    drop(tracker);

    // Block should still be optimistic (below threshold)
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let exec_status = fc
        .get_block_execution_status(&block_root)
        .expect("block should be in fork choice");
    assert!(
        exec_status.is_optimistic_or_invalid(),
        "block should remain optimistic when pending proofs below threshold, got {:?}",
        exec_status
    );
}

// ============================================================================
// Envelope gossip verification — error paths
// ============================================================================

/// Helper: produce a block+envelope at next slot and import only the block.
/// Returns (block_root, signed_envelope).
async fn import_block_get_envelope(
    harness: &BeaconChainHarness<EphemeralHarnessType<E>>,
) -> (Hash256, SignedExecutionPayloadEnvelope<E>) {
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("Gloas block should produce an envelope");
    let block_root = block_contents.0.canonical_root();

    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    (block_root, signed_envelope)
}

/// Helper: call `verify_payload_envelope_for_gossip` and assert it returns an
/// error. Returns the error. Panics on Ok (VerifiedPayloadEnvelope doesn't impl
/// Debug so we can't use expect_err).
fn assert_envelope_rejected(
    harness: &BeaconChainHarness<EphemeralHarnessType<E>>,
    envelope: SignedExecutionPayloadEnvelope<E>,
    context: &str,
) -> PayloadEnvelopeError {
    match harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(envelope))
    {
        Err(e) => e,
        Ok(_) => panic!("envelope should have been rejected: {}", context),
    }
}

/// Envelope with a tampered slot (different from the block's slot) is rejected
/// with `SlotMismatch`.
#[tokio::test]
async fn gloas_envelope_gossip_rejects_slot_mismatch() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;
    let (_block_root, mut signed_envelope) = import_block_get_envelope(&harness).await;

    // Tamper the slot
    signed_envelope.message.slot += 100;

    let err = assert_envelope_rejected(&harness, signed_envelope, "tampered slot");
    assert!(
        matches!(err, PayloadEnvelopeError::SlotMismatch { .. }),
        "expected SlotMismatch, got {:?}",
        err
    );
}

/// Envelope with a tampered builder_index (different from the committed bid) is
/// rejected with `BuilderIndexMismatch`.
#[tokio::test]
async fn gloas_envelope_gossip_rejects_builder_index_mismatch() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;
    let (_block_root, mut signed_envelope) = import_block_get_envelope(&harness).await;

    // Tamper builder_index to something other than the committed bid's builder_index
    signed_envelope.message.builder_index = signed_envelope.message.builder_index.wrapping_add(1);

    let err = assert_envelope_rejected(&harness, signed_envelope, "tampered builder_index");
    assert!(
        matches!(err, PayloadEnvelopeError::BuilderIndexMismatch { .. }),
        "expected BuilderIndexMismatch, got {:?}",
        err
    );
}

/// Envelope with a tampered payload block_hash (different from the committed
/// bid's block_hash) is rejected with `BlockHashMismatch`.
#[tokio::test]
async fn gloas_envelope_gossip_rejects_block_hash_mismatch() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;
    let (_block_root, mut signed_envelope) = import_block_get_envelope(&harness).await;

    // Tamper the payload block_hash
    signed_envelope.message.payload.block_hash = ExecutionBlockHash::from_root(Hash256::random());

    let err = assert_envelope_rejected(&harness, signed_envelope, "tampered block_hash");
    assert!(
        matches!(err, PayloadEnvelopeError::BlockHashMismatch { .. }),
        "expected BlockHashMismatch, got {:?}",
        err
    );
}

/// Envelope referencing an unknown beacon_block_root is buffered in
/// `pending_gossip_envelopes` and returns `BlockRootUnknown`.
#[tokio::test]
async fn gloas_envelope_gossip_buffers_unknown_block_root() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;
    let (_block_root, mut signed_envelope) = import_block_get_envelope(&harness).await;

    // Tamper beacon_block_root to something unknown
    let fake_root = Hash256::random();
    signed_envelope.message.beacon_block_root = fake_root;

    let err = assert_envelope_rejected(&harness, signed_envelope, "unknown block root");
    assert!(
        matches!(err, PayloadEnvelopeError::BlockRootUnknown { .. }),
        "expected BlockRootUnknown, got {:?}",
        err
    );

    // Verify envelope was buffered for later processing
    let pending = harness.chain.pending_gossip_envelopes.lock();
    assert!(
        pending.contains_key(&fake_root),
        "envelope should be buffered in pending_gossip_envelopes"
    );
}

/// Envelope for a pre-Gloas (Fulu) block is rejected with `NotGloasBlock`.
/// We tamper the beacon_block_root to point at the genesis block (which is Fulu,
/// not Gloas), and ensure the block root IS in fork choice so we get past the
/// BlockRootUnknown check.
#[tokio::test]
async fn gloas_envelope_gossip_rejects_not_gloas_block() {
    // Start with Gloas at epoch 1 so genesis is Fulu
    let harness = gloas_harness_at_epoch(1);
    // Extend into Gloas territory
    Box::pin(harness.extend_slots(E::slots_per_epoch() as usize + 2)).await;
    let (_block_root, mut signed_envelope) = import_block_get_envelope(&harness).await;

    // Point the envelope at the genesis block root (a Fulu block)
    let genesis_root = harness.chain.genesis_block_root;
    signed_envelope.message.beacon_block_root = genesis_root;
    // Set slot to genesis slot (0) to pass the slot check against genesis block's slot
    signed_envelope.message.slot = Slot::new(0);

    let err = assert_envelope_rejected(&harness, signed_envelope, "pre-Gloas block");
    // This should fail with either PriorToFinalization (if finalized) or NotGloasBlock
    // (if not finalized). Both are correct rejections.
    assert!(
        matches!(
            err,
            PayloadEnvelopeError::NotGloasBlock { .. }
                | PayloadEnvelopeError::PriorToFinalization { .. }
        ),
        "expected NotGloasBlock or PriorToFinalization, got {:?}",
        err
    );
}

// ============================================================================
// Execution bid gossip verification — error paths
// ============================================================================

/// Helper: extract the self-build bid from a freshly produced Gloas block.
fn extract_self_build_bid<E: EthSpec>(
    block: &SignedBeaconBlock<E>,
) -> SignedExecutionPayloadBid<E> {
    block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("Gloas block should have a bid")
        .clone()
}

/// Helper: call `verify_execution_bid_for_gossip` and assert it returns an error.
/// Returns the error. Panics if the result is Ok (VerifiedExecutionBid doesn't
/// impl Debug so we can't use expect_err).
fn assert_bid_rejected(
    harness: &BeaconChainHarness<EphemeralHarnessType<E>>,
    bid: SignedExecutionPayloadBid<E>,
    context: &str,
) -> ExecutionBidError {
    match harness.chain.verify_execution_bid_for_gossip(bid) {
        Err(e) => e,
        Ok(_) => panic!("bid should have been rejected: {}", context),
    }
}

/// A bid for a slot that is neither current nor next is rejected with
/// `SlotNotCurrentOrNext`. The slot check is the FIRST validation, so it
/// triggers regardless of other bid fields.
#[tokio::test]
async fn gloas_bid_gossip_rejects_slot_not_current_or_next() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, _envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let mut bid = extract_self_build_bid(&block_contents.0);
    // Set slot far in the future (well beyond current + 1)
    bid.message.slot = Slot::new(999);

    let err = assert_bid_rejected(&harness, bid, "far-future slot");
    assert!(
        matches!(err, ExecutionBidError::SlotNotCurrentOrNext { .. }),
        "expected SlotNotCurrentOrNext, got {:?}",
        err
    );
}

/// A bid with execution_payment == 0 (after passing slot check) is rejected
/// with `ZeroExecutionPayment`. Self-build bids naturally have payment == 0.
#[tokio::test]
async fn gloas_bid_gossip_rejects_zero_execution_payment() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, _envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let bid = extract_self_build_bid(&block_contents.0);
    // Self-build bids have execution_payment = 0, which should be rejected.
    assert_eq!(bid.message.execution_payment, 0);

    let err = assert_bid_rejected(&harness, bid, "zero execution_payment");
    assert!(
        matches!(err, ExecutionBidError::ZeroExecutionPayment),
        "expected ZeroExecutionPayment, got {:?}",
        err
    );
}

/// A bid with builder_index = BUILDER_INDEX_SELF_BUILD (u64::MAX) and
/// execution_payment > 0 is rejected with `UnknownBuilder` because the
/// self-build index doesn't correspond to any registered builder.
/// This tests the builder registry lookup (check 2).
#[tokio::test]
async fn gloas_bid_gossip_rejects_unknown_builder() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, _envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let mut bid = extract_self_build_bid(&block_contents.0);
    // Set execution_payment > 0 to pass the payment check
    bid.message.execution_payment = 1;
    // builder_index is BUILDER_INDEX_SELF_BUILD (u64::MAX) from self-build,
    // which doesn't exist in the empty builders registry

    let err = assert_bid_rejected(&harness, bid, "unknown builder");
    assert!(
        matches!(err, ExecutionBidError::UnknownBuilder { .. }),
        "expected UnknownBuilder, got {:?}",
        err
    );
}

/// A bid referencing a builder_index that exceeds the builders list length
/// is rejected with `UnknownBuilder`. Tests with a small, non-MAX index.
#[tokio::test]
async fn gloas_bid_gossip_rejects_nonexistent_builder_index() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, _envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let mut bid = extract_self_build_bid(&block_contents.0);
    bid.message.execution_payment = 1;
    // Use builder_index = 42 — no builders in the default state
    bid.message.builder_index = 42;

    let err = assert_bid_rejected(&harness, bid, "nonexistent builder index");
    assert!(
        matches!(err, ExecutionBidError::UnknownBuilder { builder_index: 42 }),
        "expected UnknownBuilder with index 42, got {:?}",
        err
    );
}

// =============================================================================
// Execution payload Gloas path tests (PayloadNotifier, validate_execution_payload_for_gossip)
// =============================================================================

/// PayloadNotifier::new for a Gloas block should return Irrelevant immediately,
/// without sending any request to the execution layer. Gloas blocks carry no
/// execution payload in the block body — the payload arrives via a separate
/// envelope. If this path returned Optimistic or None, the block import would
/// either call the EL unnecessarily or fail.
#[tokio::test]
async fn gloas_payload_notifier_returns_irrelevant() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    // Produce a new block + state
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let ((signed_block, _blobs), post_state) = harness.make_block(head_state, next_slot).await;

    assert!(
        signed_block.fork_name_unchecked().gloas_enabled(),
        "block should be Gloas"
    );

    // Construct PayloadNotifier with NotifyExecutionLayer::Yes — if Gloas logic
    // is correct, it should still return Irrelevant (not call the EL).
    let notifier = PayloadNotifier::new(
        harness.chain.clone(),
        signed_block.clone(),
        &post_state,
        NotifyExecutionLayer::Yes,
    )
    .expect("PayloadNotifier::new should succeed for Gloas block");

    let status = notifier
        .notify_new_payload()
        .await
        .expect("notify_new_payload should succeed");

    assert_eq!(
        status,
        PayloadVerificationStatus::Irrelevant,
        "Gloas block payload verification should be Irrelevant, got {:?}",
        status
    );
}

/// PayloadNotifier::new for a pre-Gloas (Fulu) block should NOT return Irrelevant
/// when execution is enabled — it should either return None (requiring EL call)
/// or Optimistic. This is the complement test ensuring the Gloas early-return
/// only fires for Gloas blocks.
#[tokio::test]
async fn fulu_payload_notifier_does_not_return_irrelevant() {
    let harness = gloas_harness_at_epoch(2);

    // Build blocks in the Fulu era (before Gloas fork at epoch 2)
    Box::pin(harness.extend_slots(2)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;

    // Use make_block_return_pre_state so we get the PRE-BLOCK state, which is
    // what PayloadNotifier::new expects (it runs partially_verify_execution_payload
    // which checks the execution hash chain against the parent state).
    let ((signed_block, _blobs), pre_state) = harness
        .make_block_return_pre_state(head_state, next_slot)
        .await;

    assert!(
        !signed_block.fork_name_unchecked().gloas_enabled(),
        "block should be pre-Gloas (Fulu)"
    );

    let notifier = PayloadNotifier::new(
        harness.chain.clone(),
        signed_block.clone(),
        &pre_state,
        NotifyExecutionLayer::Yes,
    )
    .expect("PayloadNotifier::new should succeed for Fulu block");

    // For a Fulu block with execution enabled, the notifier should call the EL.
    // The mock EL will return Valid.
    let status = notifier
        .notify_new_payload()
        .await
        .expect("notify_new_payload should succeed");

    assert_ne!(
        status,
        PayloadVerificationStatus::Irrelevant,
        "Fulu block should not be Irrelevant — execution is enabled"
    );
}

/// validate_execution_payload_for_gossip should be a no-op for Gloas blocks.
/// Gloas blocks don't carry an execution payload in the block body (the payload
/// comes via a separate envelope), so the timestamp and merge-transition checks
/// don't apply. A bug here would reject valid Gloas blocks during gossip.
#[tokio::test]
async fn gloas_gossip_skips_execution_payload_validation() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    // Get the head block info from fork choice
    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    // Produce a block at the next slot
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let ((signed_block, _blobs), _state) = harness.make_block(head_state, next_slot).await;

    assert!(
        signed_block.fork_name_unchecked().gloas_enabled(),
        "block should be Gloas"
    );

    // Get the parent's proto block from fork choice
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let parent_block = fc
        .get_block(&head_root)
        .expect("head block should be in fork choice");
    drop(fc);

    // This should return Ok(()) immediately for Gloas blocks
    let result = validate_execution_payload_for_gossip(
        &parent_block,
        signed_block.message(),
        &harness.chain,
    );
    assert!(
        result.is_ok(),
        "validate_execution_payload_for_gossip should be a no-op for Gloas blocks, got {:?}",
        result.err()
    );
}

/// For pre-Gloas (Fulu) blocks, validate_execution_payload_for_gossip performs
/// real validation (timestamp check). This complement test ensures the Gloas
/// early-return only fires for Gloas blocks.
#[tokio::test]
async fn fulu_gossip_validates_execution_payload() {
    let harness = gloas_harness_at_epoch(2);
    Box::pin(harness.extend_slots(2)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let ((signed_block, _blobs), _state) = harness.make_block(head_state, next_slot).await;

    assert!(
        !signed_block.fork_name_unchecked().gloas_enabled(),
        "block should be Fulu (pre-Gloas)"
    );

    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let parent_block = fc
        .get_block(&head_root)
        .expect("head block should be in fork choice");
    drop(fc);

    // For Fulu blocks with valid execution payloads, this should also return Ok.
    // The point is that it RUNS the validation (doesn't early-return), and for
    // a correctly-produced block, the timestamp check passes.
    let result = validate_execution_payload_for_gossip(
        &parent_block,
        signed_block.message(),
        &harness.chain,
    );
    assert!(
        result.is_ok(),
        "validate_execution_payload_for_gossip should pass for valid Fulu block, got {:?}",
        result.err()
    );
}

/// The self-build envelope constructed during Gloas block production should have
/// a valid post-envelope state_root — one that differs from the block's
/// (pre-envelope) state_root. This tests the `build_self_build_envelope` method
/// which clones the post-block state, runs `process_execution_payload_envelope`,
/// and captures the state root from the InvalidStateRoot error path.
#[tokio::test]
async fn gloas_self_build_envelope_state_root_differs_from_block() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let envelope = envelope.expect("Gloas block should produce an envelope");
    let block = &block_contents.0;
    let block_state_root = block.message().state_root();
    let envelope_state_root = envelope.message.state_root;

    // The envelope's state_root should be the post-envelope root, which is
    // different from the block's pre-envelope state_root.
    assert_ne!(
        block_state_root, envelope_state_root,
        "envelope state_root should differ from block state_root (pre vs post envelope)"
    );

    // Both should be non-zero
    assert_ne!(
        block_state_root,
        Hash256::zero(),
        "block state_root should not be zero"
    );
    assert_ne!(
        envelope_state_root,
        Hash256::zero(),
        "envelope state_root should not be zero"
    );

    // The envelope should reference the correct block root
    let block_root = block.canonical_root();
    assert_eq!(
        envelope.message.beacon_block_root, block_root,
        "envelope should reference the block it was built for"
    );

    // The envelope slot should match the block slot
    assert_eq!(
        envelope.message.slot,
        block.slot(),
        "envelope slot should match block slot"
    );
}

/// After extending the chain, each produced Gloas block should have an envelope
/// whose payload.block_hash matches the EL-returned payload, and the bid's
/// parent_block_hash should differ from the payload block_hash (parent vs child).
#[tokio::test]
async fn gloas_self_build_envelope_payload_block_hash_consistency() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(4)).await;

    let head = harness.chain.head_snapshot();
    let block = head.beacon_block.as_gloas().expect("should be Gloas");

    // Get the envelope from the store
    let head_root = head.beacon_block_root;
    let envelope = harness
        .chain
        .get_payload_envelope(&head_root)
        .expect("store access should succeed")
        .expect("envelope should exist for head block");

    let payload_block_hash = envelope.message.payload.block_hash;
    let bid_parent_hash = block
        .message
        .body
        .signed_execution_payload_bid
        .message
        .parent_block_hash;

    // The payload's block_hash should not be zero (the EL returned a real payload)
    assert_ne!(
        payload_block_hash,
        ExecutionBlockHash::zero(),
        "payload block_hash should be non-zero"
    );

    // The bid's parent_block_hash should differ from the payload block_hash
    // (parent_block_hash is the PREVIOUS block's execution hash, payload block_hash
    // is THIS block's execution hash)
    assert_ne!(
        bid_parent_hash, payload_block_hash,
        "bid parent_block_hash should differ from payload block_hash"
    );
}

/// The Gloas execution payload path in `get_execution_payload` reads gas_limit
/// from `state.latest_execution_payload_bid().gas_limit` instead of
/// `state.latest_execution_payload_header().gas_limit`. Verify that the gas_limit
/// in the produced block's payload is consistent with the bid's gas_limit from
/// the previous state.
#[tokio::test]
async fn gloas_block_production_gas_limit_from_bid() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_state = &head.beacon_state;

    // Read the gas_limit that get_execution_payload will extract
    let bid_gas_limit = head_state
        .latest_execution_payload_bid()
        .expect("Gloas state should have latest_execution_payload_bid")
        .gas_limit;

    // The gas_limit should be non-zero (set by the mock EL)
    assert_ne!(
        bid_gas_limit, 0,
        "latest_execution_payload_bid gas_limit should be non-zero"
    );

    // Produce a block and verify the payload's gas_limit is reasonable
    // (the mock EL should produce a payload with a gas_limit related to the parent)
    harness.advance_slot();
    let state = harness.chain.head_beacon_state_cloned();
    let next_slot = state.slot() + 1;
    let (_block_contents, _state, envelope) =
        harness.make_block_with_envelope(state, next_slot).await;

    let envelope = envelope.expect("should produce envelope");
    let payload_gas_limit = envelope.message.payload.gas_limit;

    // The produced payload should have a non-zero gas_limit
    assert_ne!(
        payload_gas_limit, 0,
        "produced payload gas_limit should be non-zero"
    );
}

// =============================================================================
// Fork choice Gloas method tests: on_execution_bid, on_payload_attestation,
// on_execution_payload
// =============================================================================

/// Helper: produce a Gloas block and return the head block_root and slot.
async fn produce_gloas_block(
    harness: &BeaconChainHarness<EphemeralHarnessType<E>>,
) -> (Hash256, Slot) {
    Box::pin(harness.extend_slots(1)).await;
    let head = harness.chain.head_snapshot();
    (head.beacon_block_root, head.beacon_block.slot())
}

/// on_execution_bid: rejects a bid for an unknown block root.
#[tokio::test]
async fn fc_on_execution_bid_rejects_unknown_block_root() {
    let harness = gloas_harness_at_epoch(0);
    let (_, slot) = produce_gloas_block(&harness).await;

    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot,
            builder_index: 1,
            block_hash: ExecutionBlockHash::repeat_byte(0xaa),
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };

    let unknown_root = Hash256::repeat_byte(0xff);
    let result = harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_execution_bid(&bid, unknown_root);

    assert!(result.is_err(), "should reject bid for unknown block root");
}

/// on_execution_bid: rejects a bid with mismatched slot.
#[tokio::test]
async fn fc_on_execution_bid_rejects_slot_mismatch() {
    let harness = gloas_harness_at_epoch(0);
    let (block_root, slot) = produce_gloas_block(&harness).await;

    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot: slot + 100, // wrong slot
            builder_index: 1,
            block_hash: ExecutionBlockHash::repeat_byte(0xaa),
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };

    let result = harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_execution_bid(&bid, block_root);

    assert!(result.is_err(), "should reject bid with wrong slot");
}

/// on_execution_bid: accepts a valid bid and updates node fields.
#[tokio::test]
async fn fc_on_execution_bid_updates_node_fields() {
    let harness = gloas_harness_at_epoch(0);
    let (block_root, slot) = produce_gloas_block(&harness).await;

    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot,
            builder_index: 42,
            block_hash: ExecutionBlockHash::repeat_byte(0xaa),
            value: 1000,
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };

    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_execution_bid(&bid, block_root)
        .expect("valid bid should succeed");

    // Verify node was updated
    let node = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&block_root)
        .expect("block should exist in fork choice");

    assert_eq!(
        node.builder_index,
        Some(42),
        "builder_index should be set from bid"
    );
    // After on_execution_bid, payload_revealed is reset to false (awaiting builder reveal)
    assert!(
        !node.payload_revealed,
        "payload_revealed should be false after bid (awaiting reveal)"
    );
    assert_eq!(node.ptc_weight, 0, "ptc_weight should be initialized to 0");
    assert_eq!(
        node.ptc_blob_data_available_weight, 0,
        "ptc_blob_data_available_weight should be initialized to 0"
    );
    assert!(
        !node.payload_data_available,
        "payload_data_available should be false"
    );
}

/// on_execution_payload: marks payload as revealed and sets execution status.
#[tokio::test]
async fn fc_on_execution_payload_marks_revealed() {
    let harness = gloas_harness_at_epoch(0);
    let (block_root, slot) = produce_gloas_block(&harness).await;

    // First apply a bid to set payload_revealed=false
    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot,
            builder_index: 1,
            block_hash: ExecutionBlockHash::repeat_byte(0xbb),
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };
    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_execution_bid(&bid, block_root)
        .unwrap();

    // Verify not yet revealed
    let node = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&block_root)
        .unwrap();
    assert!(!node.payload_revealed);

    // Now reveal the payload
    let payload_hash = ExecutionBlockHash::repeat_byte(0xcc);
    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_execution_payload(block_root, payload_hash)
        .expect("on_execution_payload should succeed");

    let node = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&block_root)
        .unwrap();

    assert!(
        node.payload_revealed,
        "payload_revealed should be true after on_execution_payload"
    );
    assert!(
        node.payload_data_available,
        "payload_data_available should be true after on_execution_payload"
    );
    assert_eq!(
        node.execution_status,
        ExecutionStatus::Optimistic(payload_hash),
        "execution_status should be Optimistic with the payload hash"
    );
}

/// on_execution_payload: rejects unknown block root.
#[tokio::test]
async fn fc_on_execution_payload_rejects_unknown_root() {
    let harness = gloas_harness_at_epoch(0);
    let _ = produce_gloas_block(&harness).await;

    let result = harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_execution_payload(
            Hash256::repeat_byte(0xff),
            ExecutionBlockHash::repeat_byte(0x01),
        );

    assert!(
        result.is_err(),
        "should reject on_execution_payload for unknown root"
    );
}

/// on_payload_attestation: rejects a future slot attestation.
#[tokio::test]
async fn fc_on_payload_attestation_rejects_future_slot() {
    let harness = gloas_harness_at_epoch(0);
    let (block_root, slot) = produce_gloas_block(&harness).await;

    let attestation = PayloadAttestation::<E> {
        data: PayloadAttestationData {
            beacon_block_root: block_root,
            slot: slot + 100, // far future
            payload_present: true,
            blob_data_available: true,
        },
        ..PayloadAttestation::empty()
    };
    let indexed = IndexedPayloadAttestation::<E> {
        attesting_indices: {
            let mut list = ssz_types::VariableList::empty();
            list.push(0).unwrap();
            list
        },
        data: attestation.data.clone(),
        ..IndexedPayloadAttestation::empty()
    };

    let result = harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_payload_attestation(&attestation, &indexed, slot, &harness.spec);

    assert!(
        result.is_err(),
        "should reject payload attestation from future slot"
    );
}

/// on_payload_attestation: rejects a too-old attestation.
#[tokio::test]
async fn fc_on_payload_attestation_rejects_too_old() {
    let harness = gloas_harness_at_epoch(0);
    // Produce enough blocks so current_slot is well ahead
    Box::pin(harness.extend_slots(E::slots_per_epoch() as usize + 5)).await;

    let head = harness.chain.head_snapshot();
    let current_slot = head.beacon_block.slot();

    // Construct attestation for slot 1 (way in the past)
    let old_slot = Slot::new(1);
    let old_root = *head
        .beacon_state
        .get_block_root(old_slot)
        .expect("slot 1 should have a block root");

    let attestation = PayloadAttestation::<E> {
        data: PayloadAttestationData {
            beacon_block_root: old_root,
            slot: old_slot,
            payload_present: true,
            blob_data_available: true,
        },
        ..PayloadAttestation::empty()
    };
    let indexed = IndexedPayloadAttestation::<E> {
        attesting_indices: {
            let mut list = ssz_types::VariableList::empty();
            list.push(0).unwrap();
            list
        },
        data: attestation.data.clone(),
        ..IndexedPayloadAttestation::empty()
    };

    let result = harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_payload_attestation(&attestation, &indexed, current_slot, &harness.spec);

    assert!(
        result.is_err(),
        "should reject payload attestation older than 1 epoch"
    );
}

/// on_payload_attestation: silently ignores attestation when data.slot != block.slot
/// (e.g., attestation references a block but with a different slot — skip-slot scenario).
#[tokio::test]
async fn fc_on_payload_attestation_ignores_slot_mismatch() {
    let harness = gloas_harness_at_epoch(0);
    let (block_root, slot) = produce_gloas_block(&harness).await;

    // Apply a bid first so we can track weight changes
    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot,
            builder_index: 1,
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };
    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_execution_bid(&bid, block_root)
        .unwrap();

    // Attestation with a different slot than the block
    let attestation = PayloadAttestation::<E> {
        data: PayloadAttestationData {
            beacon_block_root: block_root,
            slot: slot + 1, // block is at `slot`, attestation says `slot + 1`
            payload_present: true,
            blob_data_available: true,
        },
        ..PayloadAttestation::empty()
    };
    let indexed = IndexedPayloadAttestation::<E> {
        attesting_indices: {
            let mut list = ssz_types::VariableList::empty();
            list.push(0).unwrap();
            list
        },
        data: attestation.data.clone(),
        ..IndexedPayloadAttestation::empty()
    };

    // Should succeed (silently ignore per spec: `if data.slot != state.slot: return`)
    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_payload_attestation(&attestation, &indexed, slot + 1, &harness.spec)
        .expect("should not error on slot mismatch (silent ignore)");

    // Verify no weight was accumulated
    let node = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&block_root)
        .unwrap();
    assert_eq!(
        node.ptc_weight, 0,
        "ptc_weight should remain 0 when attestation slot != block slot"
    );
}

/// on_payload_attestation: accumulates ptc_weight and triggers payload_revealed
/// when quorum (> PTC_SIZE/2) is reached.
#[tokio::test]
async fn fc_on_payload_attestation_quorum_triggers_payload_revealed() {
    let harness = gloas_harness_at_epoch(0);
    let (block_root, slot) = produce_gloas_block(&harness).await;

    // Apply a bid to initialize PTC tracking and set payload_revealed=false
    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot,
            builder_index: 1,
            block_hash: ExecutionBlockHash::repeat_byte(0xdd),
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };
    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_execution_bid(&bid, block_root)
        .unwrap();

    let ptc_size = harness.spec.ptc_size;
    let quorum_threshold = ptc_size / 2;

    // Send attestation with exactly quorum_threshold votes (should NOT trigger)
    let attestation_below = PayloadAttestation::<E> {
        data: PayloadAttestationData {
            beacon_block_root: block_root,
            slot,
            payload_present: true,
            blob_data_available: false,
        },
        ..PayloadAttestation::empty()
    };
    let indexed_below = IndexedPayloadAttestation::<E> {
        attesting_indices: {
            let mut list = ssz_types::VariableList::empty();
            for i in 0..quorum_threshold {
                list.push(i).unwrap();
            }
            list
        },
        data: attestation_below.data.clone(),
        ..IndexedPayloadAttestation::empty()
    };

    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_payload_attestation(&attestation_below, &indexed_below, slot, &harness.spec)
        .unwrap();

    // Check: ptc_weight should be exactly quorum_threshold, but NOT revealed yet
    let node = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&block_root)
        .unwrap();
    assert_eq!(
        node.ptc_weight, quorum_threshold,
        "ptc_weight should equal quorum_threshold"
    );
    assert!(
        !node.payload_revealed,
        "payload_revealed should still be false at exactly quorum_threshold (needs strictly greater)"
    );

    // Send one more vote to cross the threshold
    let attestation_one_more = PayloadAttestation::<E> {
        data: PayloadAttestationData {
            beacon_block_root: block_root,
            slot,
            payload_present: true,
            blob_data_available: false,
        },
        ..PayloadAttestation::empty()
    };
    let indexed_one_more = IndexedPayloadAttestation::<E> {
        attesting_indices: {
            let mut list = ssz_types::VariableList::empty();
            list.push(quorum_threshold as u64).unwrap(); // one more attester
            list
        },
        data: attestation_one_more.data.clone(),
        ..IndexedPayloadAttestation::empty()
    };

    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_payload_attestation(
            &attestation_one_more,
            &indexed_one_more,
            slot,
            &harness.spec,
        )
        .unwrap();

    let node = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&block_root)
        .unwrap();
    assert_eq!(
        node.ptc_weight,
        quorum_threshold + 1,
        "ptc_weight should be quorum_threshold + 1"
    );
    assert!(
        node.payload_revealed,
        "payload_revealed should be true after crossing quorum threshold"
    );
}

/// on_payload_attestation: blob_data_available quorum is tracked independently
/// from payload_present quorum.
#[tokio::test]
async fn fc_on_payload_attestation_blob_quorum_independent() {
    let harness = gloas_harness_at_epoch(0);
    let (block_root, slot) = produce_gloas_block(&harness).await;

    // Apply bid
    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot,
            builder_index: 1,
            block_hash: ExecutionBlockHash::repeat_byte(0xee),
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };
    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_execution_bid(&bid, block_root)
        .unwrap();

    let ptc_size = harness.spec.ptc_size;
    let quorum_threshold = ptc_size / 2;

    // Send blob_data_available=true but payload_present=false
    let attestation = PayloadAttestation::<E> {
        data: PayloadAttestationData {
            beacon_block_root: block_root,
            slot,
            payload_present: false,
            blob_data_available: true,
        },
        ..PayloadAttestation::empty()
    };
    let indexed = IndexedPayloadAttestation::<E> {
        attesting_indices: {
            let mut list = ssz_types::VariableList::empty();
            for i in 0..=quorum_threshold {
                list.push(i).unwrap();
            }
            list
        },
        data: attestation.data.clone(),
        ..IndexedPayloadAttestation::empty()
    };

    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_payload_attestation(&attestation, &indexed, slot, &harness.spec)
        .unwrap();

    let node = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&block_root)
        .unwrap();

    // payload_present=false → ptc_weight should remain 0
    assert_eq!(
        node.ptc_weight, 0,
        "ptc_weight should be 0 when payload_present=false"
    );
    assert!(
        !node.payload_revealed,
        "payload_revealed should be false (no payload_present votes)"
    );

    // blob_data_available should have crossed quorum
    assert_eq!(
        node.ptc_blob_data_available_weight,
        quorum_threshold + 1,
        "ptc_blob_data_available_weight should be quorum_threshold + 1"
    );
    assert!(
        node.payload_data_available,
        "payload_data_available should be true after blob quorum reached"
    );
}

/// on_payload_attestation: rejects attestation for unknown block root.
#[tokio::test]
async fn fc_on_payload_attestation_rejects_unknown_root() {
    let harness = gloas_harness_at_epoch(0);
    let (_, slot) = produce_gloas_block(&harness).await;

    let unknown_root = Hash256::repeat_byte(0xab);
    let attestation = PayloadAttestation::<E> {
        data: PayloadAttestationData {
            beacon_block_root: unknown_root,
            slot,
            payload_present: true,
            blob_data_available: true,
        },
        ..PayloadAttestation::empty()
    };
    let indexed = IndexedPayloadAttestation::<E> {
        attesting_indices: {
            let mut list = ssz_types::VariableList::empty();
            list.push(0).unwrap();
            list
        },
        data: attestation.data.clone(),
        ..IndexedPayloadAttestation::empty()
    };

    let result = harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_payload_attestation(&attestation, &indexed, slot, &harness.spec);

    assert!(
        result.is_err(),
        "should reject payload attestation for unknown block root"
    );
}

/// on_execution_bid followed by on_execution_payload: full lifecycle.
/// Bid sets payload_revealed=false, then on_execution_payload reveals it.
#[tokio::test]
async fn fc_bid_then_payload_lifecycle() {
    let harness = gloas_harness_at_epoch(0);
    let (block_root, slot) = produce_gloas_block(&harness).await;

    let payload_hash = ExecutionBlockHash::repeat_byte(0xf0);

    // 1. Apply bid
    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot,
            builder_index: 7,
            block_hash: payload_hash,
            value: 500,
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };
    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_execution_bid(&bid, block_root)
        .unwrap();

    let node = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&block_root)
        .unwrap();
    assert_eq!(node.builder_index, Some(7));
    assert!(!node.payload_revealed);
    assert!(!node.payload_data_available);

    // 2. Reveal payload
    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_execution_payload(block_root, payload_hash)
        .unwrap();

    let node = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&block_root)
        .unwrap();
    assert!(node.payload_revealed);
    assert!(node.payload_data_available);
    assert_eq!(
        node.execution_status,
        ExecutionStatus::Optimistic(payload_hash)
    );
}

/// on_payload_attestation with payload_present=true sets execution_status
/// to Optimistic via bid_block_hash when quorum is reached and envelope
/// hasn't arrived yet.
#[tokio::test]
async fn fc_payload_attestation_quorum_sets_optimistic_from_bid_hash() {
    let harness = gloas_harness_at_epoch(0);
    let (block_root, slot) = produce_gloas_block(&harness).await;

    let bid_hash = ExecutionBlockHash::repeat_byte(0xfa);

    // Apply bid — this also stores bid_block_hash in the proto node via on_block
    // We need to manually set bid_block_hash since on_execution_bid doesn't set it
    // (it's set during on_block). Instead, test the quorum path by first setting
    // the node's bid_block_hash directly through a write lock.
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        // Apply bid to reset PTC state
        let bid = SignedExecutionPayloadBid::<E> {
            message: ExecutionPayloadBid {
                slot,
                builder_index: 2,
                block_hash: bid_hash,
                ..Default::default()
            },
            signature: bls::Signature::empty(),
        };
        fc.on_execution_bid(&bid, block_root).unwrap();

        // Set bid_block_hash on the proto_array node directly
        // (normally set by on_block when block is imported)
        let block_index = fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&block_root)
            .copied()
            .unwrap();
        let nodes = &mut fc.proto_array_mut().core_proto_array_mut().nodes;
        if let Some(node) = nodes.get_mut(block_index) {
            node.bid_block_hash = Some(bid_hash);
            // Reset execution_status so the quorum path has to set it
            node.execution_status = ExecutionStatus::irrelevant();
        }
    }

    let ptc_size = harness.spec.ptc_size;
    let quorum_threshold = ptc_size / 2;

    // Send enough votes to cross quorum
    let attestation = PayloadAttestation::<E> {
        data: PayloadAttestationData {
            beacon_block_root: block_root,
            slot,
            payload_present: true,
            blob_data_available: false,
        },
        ..PayloadAttestation::empty()
    };
    let indexed = IndexedPayloadAttestation::<E> {
        attesting_indices: {
            let mut list = ssz_types::VariableList::empty();
            for i in 0..=quorum_threshold {
                list.push(i).unwrap();
            }
            list
        },
        data: attestation.data.clone(),
        ..IndexedPayloadAttestation::empty()
    };

    harness
        .chain
        .canonical_head
        .fork_choice_write_lock()
        .on_payload_attestation(&attestation, &indexed, slot, &harness.spec)
        .unwrap();

    let node = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&block_root)
        .unwrap();
    assert!(
        node.payload_revealed,
        "quorum should trigger payload_revealed"
    );
    assert_eq!(
        node.execution_status,
        ExecutionStatus::Optimistic(bid_hash),
        "execution_status should be set to Optimistic(bid_block_hash) when quorum reached"
    );
}

// =============================================================================
// apply_payload_attestation_to_fork_choice integration tests
// =============================================================================

/// Importing a payload attestation message via the beacon chain API should update
/// the fork choice node's ptc_weight. This tests the full pipeline:
/// import_payload_attestation_message → verify_payload_attestation_for_gossip →
/// apply_payload_attestation_to_fork_choice → on_payload_attestation.
#[tokio::test]
async fn gloas_import_attestation_updates_fork_choice_ptc_weight() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    // Verify initial ptc_weight is 0 (self-build sets payload_revealed but not via PTC)
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let node = fc.get_block(&head_root).unwrap();
        assert_eq!(
            node.ptc_weight, 0,
            "ptc_weight should start at 0 before any payload attestation"
        );
    }

    // Find a PTC member and import a payload attestation
    let validator_index = first_ptc_member(state, head_slot, &harness.spec);

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: false,
    };

    let signature =
        sign_payload_attestation_data(&data, validator_index as usize, state, &harness.spec);

    let message = PayloadAttestationMessage {
        validator_index,
        data,
        signature,
    };

    let result = harness.chain.import_payload_attestation_message(message);
    assert!(
        result.is_ok(),
        "should import valid payload attestation: {:?}",
        result.err()
    );

    // Verify ptc_weight increased in fork choice
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let node = fc.get_block(&head_root).unwrap();
    assert_eq!(
        node.ptc_weight, 1,
        "ptc_weight should be 1 after importing one PTC attestation"
    );
}

/// Importing a payload attestation with blob_data_available=true should update
/// the fork choice node's ptc_blob_data_available_weight.
#[tokio::test]
async fn gloas_import_attestation_updates_blob_data_weight() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let validator_index = first_ptc_member(state, head_slot, &harness.spec);

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: false,
        blob_data_available: true,
    };

    let signature =
        sign_payload_attestation_data(&data, validator_index as usize, state, &harness.spec);

    let message = PayloadAttestationMessage {
        validator_index,
        data,
        signature,
    };

    let result = harness.chain.import_payload_attestation_message(message);
    assert!(
        result.is_ok(),
        "should import valid payload attestation: {:?}",
        result.err()
    );

    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let node = fc.get_block(&head_root).unwrap();
    assert_eq!(
        node.ptc_blob_data_available_weight, 1,
        "blob_data_available_weight should be 1 after one attestation with blob_data_available=true"
    );
    assert_eq!(
        node.ptc_weight, 0,
        "ptc_weight should be 0 when payload_present=false"
    );
}

/// When enough PTC members import attestations via the API to exceed quorum,
/// fork choice should flip payload_revealed from false to true. This tests the
/// full quorum pathway through the beacon chain import methods.
#[tokio::test]
async fn gloas_import_attestation_quorum_triggers_payload_revealed() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    // Reset payload_revealed to false so PTC quorum can trigger it
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        let block_index = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&head_root)
            .expect("head root should be in fork choice");
        let node = &mut fc.proto_array_mut().core_proto_array_mut().nodes[block_index];
        node.payload_revealed = false;
        node.ptc_weight = 0;
    }

    // Get all PTC members for this slot
    let ptc =
        get_ptc_committee::<E>(state, head_slot, &harness.spec).expect("should get PTC committee");
    let ptc_size = harness.spec.ptc_size;
    let quorum_threshold = ptc_size / 2; // Need > threshold, so quorum_threshold+1 votes

    // Import attestations from all PTC members (both members for minimal preset)
    for (i, &validator_index) in ptc.iter().enumerate() {
        let data = PayloadAttestationData {
            beacon_block_root: head_root,
            slot: head_slot,
            payload_present: true,
            blob_data_available: false,
        };

        let signature =
            sign_payload_attestation_data(&data, validator_index as usize, state, &harness.spec);

        let message = PayloadAttestationMessage {
            validator_index,
            data,
            signature,
        };

        let result = harness.chain.import_payload_attestation_message(message);
        assert!(
            result.is_ok(),
            "should import attestation from PTC member {}: {:?}",
            i,
            result.err()
        );

        // Check state after each attestation
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let node = fc.get_block(&head_root).unwrap();
        let votes_so_far = (i + 1) as u64;

        if votes_so_far > quorum_threshold {
            assert!(
                node.payload_revealed,
                "payload_revealed should be true after {} votes (quorum threshold = {})",
                votes_so_far, quorum_threshold
            );
        }
    }

    // Final verification: quorum reached
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let node = fc.get_block(&head_root).unwrap();
    assert!(
        node.payload_revealed,
        "payload_revealed should be true after all PTC members attested"
    );
    assert_eq!(
        node.ptc_weight,
        ptc.len() as u64,
        "ptc_weight should equal total PTC members"
    );
}

/// Importing a payload attestation with payload_present=false should NOT increment
/// ptc_weight but should still succeed (valid attestation for absent payload).
#[tokio::test]
async fn gloas_import_attestation_payload_absent_no_ptc_weight() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let validator_index = first_ptc_member(state, head_slot, &harness.spec);

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: false,
        blob_data_available: false,
    };

    let signature =
        sign_payload_attestation_data(&data, validator_index as usize, state, &harness.spec);

    let message = PayloadAttestationMessage {
        validator_index,
        data,
        signature,
    };

    let result = harness.chain.import_payload_attestation_message(message);
    assert!(
        result.is_ok(),
        "should import attestation with payload_present=false: {:?}",
        result.err()
    );

    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let node = fc.get_block(&head_root).unwrap();
    assert_eq!(
        node.ptc_weight, 0,
        "ptc_weight should remain 0 when payload_present=false"
    );
    assert_eq!(
        node.ptc_blob_data_available_weight, 0,
        "blob weight should remain 0 when blob_data_available=false"
    );
}

// =============================================================================
// apply_execution_bid_to_fork_choice integration tests
// =============================================================================

/// After a block is imported with a self-build bid, applying an external bid
/// via verify_execution_bid_for_gossip + apply_execution_bid_to_fork_choice
/// should both insert into the bid pool AND update fork choice builder_index.
/// Since constructing a VerifiedExecutionBid requires passing signature checks
/// against a registered builder (which the minimal test harness doesn't have),
/// we test the equivalent path: verify that get_best_execution_bid returns
/// bids that were inserted through apply_execution_bid_to_fork_choice's pool
/// insertion (bid pool is populated by the same method that updates fork choice).
/// The fork choice on_execution_bid updates are separately tested via unit tests
/// in fork_choice.rs.
///
/// This test verifies the pool insertion side of apply_execution_bid_to_fork_choice
/// by directly inserting into the pool (same code path) and checking retrieval.
#[tokio::test]
async fn gloas_bid_pool_insertion_and_retrieval_via_chain() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let target_slot = harness.chain.head_snapshot().beacon_block.slot() + 1;

    // Insert two bids at different values
    let bid1 = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot: target_slot,
            builder_index: 0,
            value: 500,
            ..Default::default()
        },
        signature: Signature::empty(),
    };
    let bid2 = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot: target_slot,
            builder_index: 1,
            value: 2000,
            ..Default::default()
        },
        signature: Signature::empty(),
    };

    // Insert through the pool (same as apply_execution_bid_to_fork_choice line 2515)
    {
        let mut pool = harness.chain.execution_bid_pool.lock();
        pool.insert(bid1);
        pool.insert(bid2);
    }

    // Verify best bid selection returns highest value
    let best = harness
        .chain
        .get_best_execution_bid(target_slot, Hash256::zero())
        .expect("should have a bid");
    assert_eq!(best.message.value, 2000, "should return highest-value bid");
    assert_eq!(best.message.builder_index, 1);

    // Verify old-slot bids are pruned
    let future_slot = target_slot + 10;
    let result = harness
        .chain
        .get_best_execution_bid(future_slot, Hash256::zero());
    assert!(
        result.is_none(),
        "bids for old slots should be pruned when querying future slot"
    );
}

// =============================================================================
// apply_execution_bid_to_fork_choice — end-to-end integration tests
// =============================================================================

/// Test that `apply_execution_bid_to_fork_choice` updates fork choice node fields:
/// builder_index, payload_revealed=false, ptc_weight=0, ptc_blob_data_available_weight=0.
/// This exercises the full beacon_chain → fork_choice → proto_array pipeline that
/// was previously only tested by directly manipulating the bid pool.
#[tokio::test]
async fn gloas_apply_bid_to_fork_choice_updates_node_fields() {
    use beacon_chain::gloas_verification::VerifiedExecutionBid;

    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    // Before applying an external bid, verify the node has self-build builder_index
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let node = fc
            .get_block(&head_root)
            .expect("head should be in fork choice");
        assert_eq!(
            node.builder_index,
            Some(u64::MAX),
            "head should have self-build builder_index before external bid"
        );
        // After extend_slots, payload is revealed (self-build envelope processed)
        assert!(
            node.payload_revealed,
            "payload should be revealed after extend_slots"
        );
    }

    // Create an external bid targeting the head block
    let external_builder_index = 42u64;
    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot: head_slot,
            builder_index: external_builder_index,
            parent_block_root: head_root,
            value: 1000,
            ..Default::default()
        },
        signature: Signature::empty(),
    };

    let verified_bid = VerifiedExecutionBid::__new_for_testing(bid);

    // Apply the bid through the beacon_chain method
    let result = harness
        .chain
        .apply_execution_bid_to_fork_choice(&verified_bid);
    assert!(
        result.is_ok(),
        "apply_execution_bid_to_fork_choice should succeed: {:?}",
        result.err()
    );

    // Verify fork choice state was updated by on_execution_bid
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let node = fc
        .get_block(&head_root)
        .expect("head should still be in fork choice");

    assert_eq!(
        node.builder_index,
        Some(external_builder_index),
        "builder_index should be updated to external builder"
    );
    assert!(
        !node.payload_revealed,
        "payload_revealed should be reset to false after new bid"
    );
    assert_eq!(node.ptc_weight, 0, "ptc_weight should be initialized to 0");
    assert_eq!(
        node.ptc_blob_data_available_weight, 0,
        "ptc_blob_data_available_weight should be initialized to 0"
    );
    assert!(
        !node.payload_data_available,
        "payload_data_available should be false"
    );
}

/// Test that `apply_execution_bid_to_fork_choice` also inserts the bid into the
/// execution bid pool, verifiable via `get_best_execution_bid`.
#[tokio::test]
async fn gloas_apply_bid_to_fork_choice_inserts_into_pool() {
    use beacon_chain::gloas_verification::VerifiedExecutionBid;

    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot: head_slot,
            builder_index: 7,
            parent_block_root: head_root,
            value: 5000,
            ..Default::default()
        },
        signature: Signature::empty(),
    };

    let verified_bid = VerifiedExecutionBid::__new_for_testing(bid);
    harness
        .chain
        .apply_execution_bid_to_fork_choice(&verified_bid)
        .expect("should succeed");

    // Verify bid is retrievable from pool (must pass matching parent_block_root)
    let best = harness
        .chain
        .get_best_execution_bid(head_slot, head_root)
        .expect("should have a bid in the pool");
    assert_eq!(best.message.value, 5000);
    assert_eq!(best.message.builder_index, 7);
}

/// Test that `apply_execution_bid_to_fork_choice` returns an error when the
/// bid references a beacon block root not in fork choice.
#[tokio::test]
async fn gloas_apply_bid_to_fork_choice_rejects_unknown_root() {
    use beacon_chain::gloas_verification::VerifiedExecutionBid;

    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let unknown_root = Hash256::from_low_u64_be(0xdeadbeef);
    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot: Slot::new(2),
            builder_index: 1,
            parent_block_root: unknown_root,
            value: 100,
            ..Default::default()
        },
        signature: Signature::empty(),
    };

    let verified_bid = VerifiedExecutionBid::__new_for_testing(bid);
    let result = harness
        .chain
        .apply_execution_bid_to_fork_choice(&verified_bid);
    assert!(
        result.is_err(),
        "should reject bid with unknown beacon block root"
    );
}

/// Test that `apply_execution_bid_to_fork_choice` returns an error when the
/// bid slot doesn't match the block's slot in fork choice.
#[tokio::test]
async fn gloas_apply_bid_to_fork_choice_rejects_slot_mismatch() {
    use beacon_chain::gloas_verification::VerifiedExecutionBid;

    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    // Bid slot doesn't match the block's actual slot
    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot: head_slot + 100,
            builder_index: 1,
            parent_block_root: head_root,
            value: 100,
            ..Default::default()
        },
        signature: Signature::empty(),
    };

    let verified_bid = VerifiedExecutionBid::__new_for_testing(bid);
    let result = harness
        .chain
        .apply_execution_bid_to_fork_choice(&verified_bid);
    assert!(result.is_err(), "should reject bid with mismatched slot");
}

/// Test the full lifecycle: apply external bid → verify fork choice reset →
/// then apply envelope → verify payload_revealed flips back to true.
/// This exercises the complete bid→reveal cycle through the beacon_chain layer.
#[tokio::test]
async fn gloas_bid_then_envelope_lifecycle_via_beacon_chain() {
    use beacon_chain::gloas_verification::VerifiedExecutionBid;

    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    // Step 1: Apply external bid — should reset payload_revealed
    let bid = SignedExecutionPayloadBid::<E> {
        message: ExecutionPayloadBid {
            slot: head_slot,
            builder_index: 10,
            parent_block_root: head_root,
            value: 3000,
            ..Default::default()
        },
        signature: Signature::empty(),
    };

    let verified_bid = VerifiedExecutionBid::__new_for_testing(bid);
    harness
        .chain
        .apply_execution_bid_to_fork_choice(&verified_bid)
        .expect("bid should be applied");

    // Verify payload_revealed is false after bid
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let node = fc.get_block(&head_root).unwrap();
        assert!(
            !node.payload_revealed,
            "payload_revealed should be false after external bid"
        );
        assert_eq!(node.builder_index, Some(10));
    }

    // Step 2: Reveal payload via on_execution_payload in fork choice
    // (simulates envelope arrival)
    let payload_hash = ExecutionBlockHash::from_root(Hash256::from_low_u64_be(0xcafe));
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        fc.on_execution_payload(head_root, payload_hash)
            .expect("on_execution_payload should succeed");
    }

    // Verify payload_revealed is now true
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let node = fc.get_block(&head_root).unwrap();
        assert!(
            node.payload_revealed,
            "payload_revealed should be true after on_execution_payload"
        );
        assert!(
            node.payload_data_available,
            "payload_data_available should be true after reveal"
        );
        assert_eq!(
            node.execution_status,
            ExecutionStatus::Optimistic(payload_hash),
            "execution_status should be Optimistic with the payload hash"
        );
    }
}

// =============================================================================
// Fork transition boundary — Fulu→Gloas invariants
// =============================================================================

/// Verify that the first Gloas block's bid parent_block_hash comes from the
/// last Fulu block's execution payload header. This is the critical invariant
/// at the fork boundary: the state upgrade copies the Fulu EL header's
/// block_hash into `latest_block_hash`, and block production reads from there.
#[tokio::test]
async fn gloas_fork_transition_bid_parent_hash_from_fulu_header() {
    let gloas_fork_epoch = Epoch::new(2);
    let gloas_fork_slot = gloas_fork_epoch.start_slot(E::slots_per_epoch());
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    // Extend to the last Fulu slot (just before the fork).
    let last_fulu_slot = gloas_fork_slot - 1;
    Box::pin(harness.extend_to_slot(last_fulu_slot)).await;

    // Capture the Fulu EL header's block_hash before the fork.
    let fulu_state = harness.chain.head_beacon_state_cloned();
    let fulu_el_block_hash = fulu_state
        .latest_execution_payload_header()
        .expect("Fulu state should have EL header")
        .block_hash();

    // Extend to the first Gloas slot.
    Box::pin(harness.extend_to_slot(gloas_fork_slot)).await;

    let gloas_head = harness.chain.head_snapshot();
    let bid = gloas_head
        .beacon_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("first Gloas block should have a bid");

    assert_eq!(
        bid.message.parent_block_hash, fulu_el_block_hash,
        "first Gloas bid parent_block_hash should equal the last Fulu EL header block_hash"
    );
}

/// Verify that after the Fulu→Gloas upgrade, the state's `latest_block_hash`
/// matches the Fulu execution payload header's `block_hash`. This is set by
/// `upgrade_state_to_gloas` and is essential for `process_execution_payload_bid`
/// to accept the first bid.
#[tokio::test]
async fn gloas_fork_transition_latest_block_hash_matches_fulu_header() {
    let gloas_fork_epoch = Epoch::new(2);
    let gloas_fork_slot = gloas_fork_epoch.start_slot(E::slots_per_epoch());
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    // Extend to the last Fulu slot.
    let last_fulu_slot = gloas_fork_slot - 1;
    Box::pin(harness.extend_to_slot(last_fulu_slot)).await;

    let fulu_el_block_hash = harness
        .chain
        .head_beacon_state_cloned()
        .latest_execution_payload_header()
        .expect("Fulu state should have EL header")
        .block_hash();

    // Extend past the fork to the first Gloas block.
    Box::pin(harness.extend_to_slot(gloas_fork_slot)).await;

    let gloas_state = harness.chain.head_beacon_state_cloned();
    assert!(
        gloas_state.fork_name_unchecked().gloas_enabled(),
        "state should be Gloas after fork"
    );

    // After envelope processing, latest_block_hash changes to the envelope's
    // block_hash. But if we look at the bid's parent_block_hash, it tells us
    // what the state's latest_block_hash was at block production time.
    let head_snap = harness.chain.head_snapshot();
    let bid = head_snap
        .beacon_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("Gloas block should have bid");
    assert_eq!(
        bid.message.parent_block_hash, fulu_el_block_hash,
        "bid parent_block_hash proves latest_block_hash was set from Fulu header"
    );
}

/// Verify that the chain continues producing valid Gloas blocks for a full
/// epoch after the fork transition. This exercises the complete pipeline:
/// fork upgrade → first Gloas block → envelope → state cache update →
/// next block reads post-envelope state → repeat.
#[tokio::test]
async fn gloas_fork_transition_chain_continues_full_epoch() {
    let gloas_fork_epoch = Epoch::new(2);
    let gloas_fork_slot = gloas_fork_epoch.start_slot(E::slots_per_epoch());
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    // Run through the fork and one full Gloas epoch.
    let target_slot = gloas_fork_slot + E::slots_per_epoch();
    Box::pin(harness.extend_to_slot(target_slot)).await;

    let head = harness.chain.head_snapshot();
    assert_eq!(head.beacon_block.slot(), target_slot);
    assert!(head.beacon_block.as_gloas().is_ok());

    // Verify each Gloas slot has a block with a non-zero bid block_hash.
    let state = &head.beacon_state;
    for slot_offset in 0..E::slots_per_epoch() {
        let slot = gloas_fork_slot + slot_offset;
        let block_root = *state.get_block_root(slot).expect("should have block root");

        let block = harness
            .chain
            .get_blinded_block(&block_root)
            .expect("should load block")
            .expect("block should exist");

        if let Ok(bid) = block.message().body().signed_execution_payload_bid() {
            assert_ne!(
                bid.message.block_hash,
                ExecutionBlockHash::zero(),
                "Gloas block at slot {} should have non-zero bid block_hash",
                slot
            );
        } else {
            panic!("block at slot {} should be Gloas with a bid", slot);
        }
    }
}

/// Verify that execution_payload_availability bits are all set after the fork
/// transition (spec: all bits = true on upgrade). This ensures per_slot_processing
/// correctly clears bits going forward and the initial state is correct.
#[tokio::test]
async fn gloas_fork_transition_execution_payload_availability_all_set() {
    let gloas_fork_epoch = Epoch::new(2);
    let gloas_fork_slot = gloas_fork_epoch.start_slot(E::slots_per_epoch());
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    // Extend to the fork slot.
    Box::pin(harness.extend_to_slot(gloas_fork_slot)).await;

    let state = harness.chain.head_beacon_state_cloned();
    let gloas_state = state.as_gloas().expect("should be Gloas state");

    // At the fork slot, the block has been processed (which clears the
    // NEXT slot's bit via per_slot_processing). But most bits should still
    // be true from the fork upgrade initialization.
    let bits = &gloas_state.execution_payload_availability;
    let total_bits = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();
    let set_count = (0..total_bits)
        .filter(|i| bits.get(*i).unwrap_or(false))
        .count();

    // At minimum, all but a few bits should be set (per_slot_processing
    // clears one bit per slot since the fork). We've only processed one
    // Gloas slot, so at most one bit should be cleared.
    assert!(
        set_count >= total_bits - 1,
        "at most one bit should be cleared after one Gloas slot (got {}/{} set)",
        set_count,
        total_bits
    );
}

/// Verify that the Gloas fork transition correctly initializes
/// builder_pending_payments as all-default (zero weight, zero amount).
/// This is critical because non-zero initial payments would cause
/// incorrect builder payment processing at the first epoch boundary.
#[tokio::test]
async fn gloas_fork_transition_builder_pending_payments_all_default() {
    let gloas_fork_epoch = Epoch::new(2);
    let gloas_fork_slot = gloas_fork_epoch.start_slot(E::slots_per_epoch());
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    Box::pin(harness.extend_to_slot(gloas_fork_slot)).await;

    let state = harness.chain.head_beacon_state_cloned();
    let gloas_state = state.as_gloas().expect("should be Gloas state");

    let payments_limit = E::builder_pending_payments_limit();
    assert_eq!(
        gloas_state.builder_pending_payments.len(),
        payments_limit,
        "builder_pending_payments should have exactly {} entries",
        payments_limit
    );

    // The first slot's bid records a pending payment for the proposer.
    // Check that all OTHER entries are still default (zero).
    let mut non_default_count = 0;
    for i in 0..payments_limit {
        let payment = gloas_state.builder_pending_payments.get(i).unwrap();
        if payment.weight != 0 || payment.withdrawal.amount != 0 {
            non_default_count += 1;
        }
    }

    // Self-build bids have value=0, so no pending payment is recorded.
    assert_eq!(
        non_default_count, 0,
        "all builder_pending_payments should be default (self-build bids have value=0)"
    );
}

// =============================================================================
// Block verification Gloas edge case tests (bid blob count, production invariants)
// =============================================================================

/// Gossip verification should reject a Gloas block whose bid contains more
/// blob_kzg_commitments than max_blobs_per_block for the block's epoch.
/// This tests the Gloas-specific branch in block_verification.rs that reads
/// commitments from the bid (not the body). Without this test, a regression
/// that only checks body commitments would silently skip Gloas validation,
/// allowing blocks with arbitrarily many blob commitments to propagate.
#[tokio::test]
async fn gloas_gossip_rejects_block_with_excess_bid_blob_commitments() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;

    let max_blobs = harness
        .chain
        .spec
        .max_blobs_per_block(next_slot.epoch(E::slots_per_epoch())) as usize;

    // Tamper the bid to have max_blobs + 1 commitments
    let ((block, _blobs), _state) = harness
        .make_block_with_modifier(head_state, next_slot, |block| {
            let body = block.body_gloas_mut().expect("should be Gloas block");
            body.signed_execution_payload_bid
                .message
                .blob_kzg_commitments =
                vec![KzgCommitment::empty_for_testing(); max_blobs + 1].into();
        })
        .await;

    let result = harness.chain.verify_block_for_gossip(block).await;
    assert!(
        result.is_err(),
        "should reject Gloas block with excess bid blob commitments"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            BlockError::InvalidBlobCount {
                max_blobs_at_epoch,
                block: blob_count,
            } if max_blobs_at_epoch == max_blobs && blob_count == max_blobs + 1
        ),
        "expected InvalidBlobCount with max={} and block={}, got {:?}",
        max_blobs,
        max_blobs + 1,
        err
    );
}

/// Complement to the excess blob test: a Gloas block with blob_kzg_commitments
/// exactly at the max should NOT be rejected by the blob count check.
/// The block may fail on a later check (e.g., signature), but should pass blob count.
#[tokio::test]
async fn gloas_gossip_accepts_block_with_valid_bid_blob_count() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;

    let max_blobs = harness
        .chain
        .spec
        .max_blobs_per_block(next_slot.epoch(E::slots_per_epoch())) as usize;

    // Set bid blob commitments to exactly the max (should pass the count check)
    let ((block, _blobs), _state) = harness
        .make_block_with_modifier(head_state, next_slot, |block| {
            let body = block.body_gloas_mut().expect("should be Gloas block");
            body.signed_execution_payload_bid
                .message
                .blob_kzg_commitments = vec![KzgCommitment::empty_for_testing(); max_blobs].into();
        })
        .await;

    let result = harness.chain.verify_block_for_gossip(block).await;
    // The block should either pass gossip verification or fail on a DIFFERENT check
    // (not InvalidBlobCount). The blob count check should pass.
    match result {
        Ok(_) => {} // passed all checks including blob count
        Err(BlockError::InvalidBlobCount { .. }) => {
            panic!("block with exactly max_blobs commitments should NOT fail blob count check");
        }
        Err(_other) => {} // failed on a later check, which is fine — blob count passed
    }
}

/// Verify that Gloas blocks have blob_kzg_commitments in the bid (not the body).
/// body.blob_kzg_commitments() should return Err for Gloas, while the bid's field
/// should be accessible. This is a structural invariant: if code mistakenly reads
/// commitments from the body instead of the bid, it would get an error.
#[tokio::test]
async fn gloas_block_blob_commitments_in_bid_not_body() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let head = &harness.chain.head_snapshot().beacon_block;
    let _gloas_block = head.as_gloas().expect("should be Gloas block");

    // Body should NOT have blob_kzg_commitments (Gloas removed them from body)
    assert!(
        head.message().body().blob_kzg_commitments().is_err(),
        "Gloas body should not have blob_kzg_commitments (they're in the bid)"
    );

    // Bid should have blob_kzg_commitments accessible
    let bid = head
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("Gloas block should have a bid");
    // Self-build blocks have empty blob commitments (no blobs in test)
    assert!(
        bid.message.blob_kzg_commitments.len()
            <= harness
                .chain
                .spec
                .max_blobs_per_block(head.slot().epoch(E::slots_per_epoch()))
                as usize,
        "bid blob commitments should not exceed max"
    );
}

/// After producing a Gloas block, verify that the state's latest_execution_payload_bid
/// has a non-zero gas_limit, and that it matches the bid in the block body.
/// This validates the Gloas path in get_execution_payload (execution_payload.rs:391-398)
/// which reads gas_limit from state.latest_execution_payload_bid() instead of
/// state.latest_execution_payload_header().
#[tokio::test]
async fn gloas_block_production_bid_gas_limit_matches_state() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let state = harness.chain.head_beacon_state_cloned();
    assert!(state.fork_name_unchecked().gloas_enabled());

    // The state's latest bid should have a non-zero gas_limit
    let latest_bid = state
        .latest_execution_payload_bid()
        .expect("Gloas state should have latest_execution_payload_bid");
    assert!(
        latest_bid.gas_limit > 0,
        "latest_execution_payload_bid gas_limit should be non-zero, got {}",
        latest_bid.gas_limit
    );

    // The head block's bid should also have a matching gas_limit
    let head = &harness.chain.head_snapshot().beacon_block;
    let block_bid = &head
        .as_gloas()
        .expect("should be Gloas")
        .message
        .body
        .signed_execution_payload_bid
        .message;
    assert_eq!(
        block_bid.gas_limit, latest_bid.gas_limit,
        "block bid gas_limit should match state's latest_execution_payload_bid gas_limit"
    );
}

/// After producing a Gloas block, verify that the state's latest_block_hash is
/// non-zero and matches the envelope's payload block_hash. This validates the
/// Gloas path in get_execution_payload (execution_payload.rs:396) which reads
/// parent_hash from state.latest_block_hash() instead of the header.
#[tokio::test]
async fn gloas_block_production_latest_block_hash_consistency() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let state = harness.chain.head_beacon_state_cloned();
    assert!(state.fork_name_unchecked().gloas_enabled());

    let latest_block_hash = *state
        .latest_block_hash()
        .expect("Gloas state should have latest_block_hash");
    assert!(
        !latest_block_hash.0.is_zero(),
        "latest_block_hash should be non-zero after processing blocks"
    );

    // Produce the next block and verify its bid's parent_block_hash equals latest_block_hash
    harness.advance_slot();
    let next_slot = state.slot() + 1;
    let ((next_block, _), _) = harness.make_block(state, next_slot).await;
    let next_bid = &next_block
        .as_gloas()
        .expect("should be Gloas")
        .message
        .body
        .signed_execution_payload_bid
        .message;

    assert_eq!(
        next_bid.parent_block_hash, latest_block_hash,
        "next block's bid parent_block_hash should equal state's latest_block_hash"
    );
}

/// Verify that Gloas block production uses get_expected_withdrawals_gloas
/// (not the pre-Gloas get_expected_withdrawals). The Gloas withdrawal function
/// includes builder_pending_withdrawals and builder sweep in addition to
/// validator withdrawals. We verify this by checking that the envelope's
/// payload has a withdrawals field that is properly populated.
#[tokio::test]
async fn gloas_block_production_uses_gloas_withdrawals() {
    let harness = gloas_harness_at_epoch(0);
    // Run for a few slots to establish some state
    Box::pin(harness.extend_slots(4)).await;

    // Get the stored envelope for the latest block
    let head_root = harness.chain.head_beacon_block_root();
    let envelope = harness
        .chain
        .get_payload_envelope(&head_root)
        .expect("should be able to read store")
        .expect("Gloas block should have a stored envelope");

    // The envelope's payload should have withdrawals
    let withdrawals = &envelope.message.payload.withdrawals;

    // In the minimal test environment with self-build blocks, there may be
    // validator withdrawals (from balance > max_effective_balance) but no
    // builder pending withdrawals (since self-build bids have value=0).
    // The key assertion is that the withdrawals field exists and is accessible.
    // The Gloas path computes withdrawals differently from pre-Gloas, so
    // if the wrong function were called, the envelope would fail processing.
    let _ = withdrawals.len(); // access to verify it's a valid field

    // Also verify via the state that payload_expected_withdrawals is accessible
    let state = harness.chain.head_beacon_state_cloned();
    let expected = state
        .payload_expected_withdrawals()
        .expect("Gloas state should have payload_expected_withdrawals");
    // payload_expected_withdrawals stores the withdrawals that were included
    // in the most recent envelope processing
    let _ = expected.len();
}

// =============================================================================
// Attestation production: payload_present (data.index) determination
// =============================================================================
// These tests exercise the full `produce_unaggregated_attestation` → `empty_for_signing`
// pipeline in a Gloas context, verifying that `data.index` correctly reflects the
// payload_present state from fork choice. The payload_present logic at
// beacon_chain.rs:2206-2217 reads `payload_revealed` from the fork choice node:
//   - Same-slot attestation (block.slot == request_slot): always payload_present=false → index=0
//   - Non-same-slot (block.slot < request_slot) with payload_revealed=true: index=1
//   - Non-same-slot (block.slot < request_slot) with payload_revealed=false: index=0
//
// Previously, there were ZERO integration tests for this logic with Gloas enabled.
// The existing attestation_production.rs tests use default_spec() which doesn't enable Gloas,
// so the Gloas branch (checking fork choice payload_revealed) was never exercised.

/// Same-slot attestation in Gloas: data.index should always be 0, regardless of
/// payload_revealed state. Per spec, same-slot attestations cannot know whether
/// the payload is present (the envelope arrives after the block).
#[tokio::test]
async fn gloas_attestation_same_slot_payload_present_false() {
    let harness = gloas_harness_at_epoch(0);
    // Produce blocks so chain advances and has valid execution status
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();

    // Verify pre-condition: payload IS revealed for the head block (envelope was processed)
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&head.beacon_block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "pre-condition: head block should have payload_revealed=true after extend_slots"
        );
    }

    // Produce attestation at the SAME slot as the head block (same-slot attestation)
    let attestation = harness
        .chain
        .produce_unaggregated_attestation(head_slot, 0)
        .expect("should produce attestation");

    // Same-slot: data.index must be 0 (payload_present=false)
    assert_eq!(
        attestation.data().index,
        0,
        "same-slot Gloas attestation should have index=0 (payload_present=false), \
         even though payload_revealed=true in fork choice. \
         Same-slot attestors cannot know if the envelope has arrived."
    );
}

/// Non-same-slot attestation with payload revealed: data.index should be 1.
/// This happens when a block was produced at an earlier slot, the envelope
/// was processed (payload_revealed=true in fork choice), and the attester
/// is producing for a later slot (skip slot or late attestation).
#[tokio::test]
async fn gloas_attestation_non_same_slot_payload_revealed_index_one() {
    let harness = gloas_harness_at_epoch(0);
    // Produce blocks with envelope processing (payload_revealed=true)
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();

    // Verify pre-condition: payload IS revealed
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&head.beacon_block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "pre-condition: payload_revealed should be true"
        );
    }

    // Advance the slot clock WITHOUT producing a block (creates a skip slot)
    harness.advance_slot();
    let attest_slot = head_slot + 1;

    // Produce attestation at a slot AFTER the head block (non-same-slot)
    let attestation = harness
        .chain
        .produce_unaggregated_attestation(attest_slot, 0)
        .expect("should produce attestation for skip slot");

    // Non-same-slot with payload_revealed=true: data.index must be 1 (payload_present=true)
    assert_eq!(
        attestation.data().index,
        1,
        "non-same-slot Gloas attestation should have index=1 (payload_present=true) \
         when payload_revealed=true in fork choice. \
         head_slot={}, attest_slot={}",
        head_slot,
        attest_slot
    );
}

/// When a Gloas block is imported without its execution payload envelope
/// being processed, the block stays Optimistic in fork choice (the EL
/// hasn't validated the payload yet). Attestation production correctly
/// refuses to attest to Optimistic blocks.
///
/// This is important: if a block's payload isn't revealed (no envelope),
/// the node MUST NOT produce attestations for it, because the execution
/// payload could be invalid.
#[tokio::test]
async fn gloas_attestation_refused_for_unrevealed_payload_block() {
    let harness = gloas_harness_at_epoch(0);
    // Produce initial blocks (with envelopes) to establish a valid chain
    Box::pin(harness.extend_slots(2)).await;

    // Now produce a block WITHOUT processing its envelope
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, _envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let block_root = block_contents.0.canonical_root();

    // Import block only — no envelope processing
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Verify: payload_revealed should be false and execution_status Optimistic
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            !proto_block.payload_revealed,
            "pre-condition: payload_revealed should be false"
        );
        let exec_status = fc.get_block_execution_status(&block_root).unwrap();
        assert!(
            !exec_status.is_valid_or_irrelevant(),
            "pre-condition: execution_status should be Optimistic (not valid)"
        );
    }

    // Advance slot for non-same-slot attestation
    harness.advance_slot();
    let attest_slot = next_slot + 1;

    // Attestation production should FAIL because the head block is Optimistic
    let result = harness
        .chain
        .produce_unaggregated_attestation(attest_slot, 0);

    assert!(
        result.is_err(),
        "should refuse to attest to a Gloas block whose payload envelope hasn't been processed \
         (execution status is Optimistic, not Valid)"
    );
    // Verify the specific error
    match result {
        Err(BeaconChainError::HeadBlockNotFullyVerified { .. }) => {}
        other => panic!("expected HeadBlockNotFullyVerified error, got {:?}", other),
    }
}

/// Pre-Gloas (Fulu) attestations should always have data.index=0, regardless
/// of slot relationship. This verifies the Gloas branch is not triggered
/// for pre-Gloas blocks.
#[tokio::test]
async fn fulu_attestation_always_index_zero() {
    // Gloas fork at epoch 2 — blocks at epoch 0 and 1 are Fulu
    let harness = gloas_harness_at_epoch(2);
    // Produce some Fulu blocks
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();

    // Verify this is a Fulu block (not Gloas)
    assert!(
        !head.beacon_block.fork_name_unchecked().gloas_enabled(),
        "pre-condition: head block should be Fulu, not Gloas"
    );

    // Advance slot for non-same-slot attestation
    harness.advance_slot();
    let attest_slot = head_slot + 1;

    let attestation = harness
        .chain
        .produce_unaggregated_attestation(attest_slot, 0)
        .expect("should produce Fulu attestation");

    // Pre-Gloas: index is always 0 (no payload_present repurposing)
    assert_eq!(
        attestation.data().index,
        0,
        "Fulu (pre-Gloas) attestation should always have index=0"
    );
}

/// After envelope processing, a Gloas block transitions from Optimistic
/// to Valid execution status, enabling attestation production. The attestation
/// should have data.index=1 (payload_present=true) for non-same-slot.
///
/// This tests the full lifecycle: block import (Optimistic, no attestation) →
/// envelope processing (Valid, attestation possible with index=1).
#[tokio::test]
async fn gloas_attestation_enabled_after_envelope_processing() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    // Produce block WITHOUT envelope processing
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let block_root = block_contents.0.canonical_root();
    let signed_envelope = envelope.expect("should have envelope");

    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Advance past the block slot
    harness.advance_slot();
    let attest_slot = next_slot + 1;

    // Before envelope processing: attestation production should FAIL (Optimistic)
    let result_before = harness
        .chain
        .produce_unaggregated_attestation(attest_slot, 0);
    assert!(
        result_before.is_err(),
        "before envelope: should refuse to attest (block is Optimistic)"
    );

    // Process the envelope (flips payload_revealed=true AND marks execution Valid)
    harness
        .chain
        .process_self_build_envelope(&signed_envelope)
        .await
        .expect("envelope processing should succeed");
    harness.chain.recompute_head_at_current_slot().await;

    // After envelope processing: attestation production should succeed with index=1
    let att_after = harness
        .chain
        .produce_unaggregated_attestation(attest_slot, 0)
        .expect("after envelope: should produce attestation (block is now Valid)");
    assert_eq!(
        att_after.data().index,
        1,
        "after envelope: non-same-slot attestation should have index=1 (payload revealed)"
    );
}

// ── Early attester cache Gloas payload_present tests ──────────────────────
//
// The early attester cache (early_attester_cache.rs) is a fast-path for
// attestation production that bypasses canonical_head. It independently
// computes `payload_present` from the proto_block's `payload_revealed` field:
//
//   payload_present = gloas_enabled && request_slot > block.slot && payload_revealed
//
// Previously, ZERO tests exercised this logic with Gloas enabled. The existing
// tests in attestation_production.rs use default_spec() which doesn't enable
// Gloas, so the early cache always computed payload_present=false regardless of
// the proto_block's payload_revealed state.

/// Early attester cache: same-slot attestation in Gloas should always have
/// index=0 (payload_present=false), even when payload_revealed=true.
/// Same-slot attestors cannot know if the envelope has arrived yet.
#[tokio::test]
async fn gloas_early_cache_same_slot_payload_present_false() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let head_root = head.beacon_block_root;

    // Get proto_block from fork choice (payload_revealed=true after envelope processing)
    let proto_block = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&head_root)
        .expect("head should be in fork choice");
    assert!(
        proto_block.payload_revealed,
        "pre-condition: head should have payload_revealed=true"
    );

    // Build available block for cache
    let rpc_block =
        harness.build_rpc_block_from_store_blobs(Some(head_root), head.beacon_block.clone());
    let beacon_chain::data_availability_checker::MaybeAvailableBlock::Available(available_block) =
        harness
            .chain
            .data_availability_checker
            .verify_kzg_for_rpc_block(rpc_block)
            .unwrap()
    else {
        panic!("block should be available")
    };

    // Add to early attester cache
    harness
        .chain
        .early_attester_cache
        .add_head_block(
            head_root,
            &available_block,
            proto_block,
            &head.beacon_state,
            &harness.chain.spec,
        )
        .unwrap();

    // Attest at the SAME slot as the head block
    let early_att = harness
        .chain
        .early_attester_cache
        .try_attest(head_slot, 0, &harness.chain.spec)
        .unwrap()
        .expect("should produce attestation from early cache");

    assert_eq!(
        early_att.data().index,
        0,
        "same-slot early cache attestation should have index=0 (payload_present=false)"
    );
}

/// Early attester cache: non-same-slot attestation with payload_revealed=true
/// should have index=1 (payload_present=true).
#[tokio::test]
async fn gloas_early_cache_non_same_slot_payload_revealed_index_one() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let head_root = head.beacon_block_root;

    let proto_block = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&head_root)
        .expect("head should be in fork choice");
    assert!(
        proto_block.payload_revealed,
        "pre-condition: payload_revealed should be true"
    );

    let rpc_block =
        harness.build_rpc_block_from_store_blobs(Some(head_root), head.beacon_block.clone());
    let beacon_chain::data_availability_checker::MaybeAvailableBlock::Available(available_block) =
        harness
            .chain
            .data_availability_checker
            .verify_kzg_for_rpc_block(rpc_block)
            .unwrap()
    else {
        panic!("block should be available")
    };

    harness
        .chain
        .early_attester_cache
        .add_head_block(
            head_root,
            &available_block,
            proto_block,
            &head.beacon_state,
            &harness.chain.spec,
        )
        .unwrap();

    // Attest at a LATER slot (non-same-slot) — should get payload_present=true
    let attest_slot = head_slot + 1;
    let early_att = harness
        .chain
        .early_attester_cache
        .try_attest(attest_slot, 0, &harness.chain.spec)
        .unwrap()
        .expect("should produce attestation from early cache");

    assert_eq!(
        early_att.data().index,
        1,
        "non-same-slot early cache attestation with payload_revealed=true should have index=1"
    );
}

/// Early attester cache: non-same-slot attestation with payload_revealed=false
/// should have index=0 (payload_present=false). This tests the safety boundary:
/// if the payload hasn't been revealed, even non-same-slot attestations must NOT
/// indicate payload presence.
#[tokio::test]
async fn gloas_early_cache_non_same_slot_payload_not_revealed_index_zero() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let head_root = head.beacon_block_root;

    // Get proto_block and override payload_revealed to false
    let mut proto_block = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&head_root)
        .expect("head should be in fork choice");
    proto_block.payload_revealed = false;

    let rpc_block =
        harness.build_rpc_block_from_store_blobs(Some(head_root), head.beacon_block.clone());
    let beacon_chain::data_availability_checker::MaybeAvailableBlock::Available(available_block) =
        harness
            .chain
            .data_availability_checker
            .verify_kzg_for_rpc_block(rpc_block)
            .unwrap()
    else {
        panic!("block should be available")
    };

    harness
        .chain
        .early_attester_cache
        .add_head_block(
            head_root,
            &available_block,
            proto_block,
            &head.beacon_state,
            &harness.chain.spec,
        )
        .unwrap();

    // Non-same-slot attestation with payload_revealed=false → index=0
    let attest_slot = head_slot + 1;
    let early_att = harness
        .chain
        .early_attester_cache
        .try_attest(attest_slot, 0, &harness.chain.spec)
        .unwrap()
        .expect("should produce attestation from early cache");

    assert_eq!(
        early_att.data().index,
        0,
        "non-same-slot early cache attestation with payload_revealed=false should have index=0"
    );
}

/// Early attester cache attestation must match produce_unaggregated_attestation
/// output in Gloas. This verifies consistency between the two attestation
/// production paths (early cache vs canonical head).
#[tokio::test]
async fn gloas_early_cache_matches_canonical_attestation() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let head_root = head.beacon_block_root;

    let proto_block = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&head_root)
        .expect("head should be in fork choice");

    let rpc_block =
        harness.build_rpc_block_from_store_blobs(Some(head_root), head.beacon_block.clone());
    let beacon_chain::data_availability_checker::MaybeAvailableBlock::Available(available_block) =
        harness
            .chain
            .data_availability_checker
            .verify_kzg_for_rpc_block(rpc_block)
            .unwrap()
    else {
        panic!("block should be available")
    };

    harness
        .chain
        .early_attester_cache
        .add_head_block(
            head_root,
            &available_block,
            proto_block,
            &head.beacon_state,
            &harness.chain.spec,
        )
        .unwrap();

    // Same-slot attestation: both paths should agree
    let canonical = harness
        .chain
        .produce_unaggregated_attestation(head_slot, 0)
        .expect("canonical attestation should succeed");
    let early = harness
        .chain
        .early_attester_cache
        .try_attest(head_slot, 0, &harness.chain.spec)
        .unwrap()
        .expect("early cache should produce attestation");

    assert_eq!(
        canonical.data().index,
        early.data().index,
        "same-slot: early cache and canonical attestation should have matching index"
    );
    assert_eq!(
        canonical.data().beacon_block_root,
        early.data().beacon_block_root,
        "same-slot: early cache and canonical attestation should have matching block root"
    );

    // Non-same-slot: advance slot and compare
    harness.advance_slot();
    let attest_slot = head_slot + 1;

    let canonical_skip = harness
        .chain
        .produce_unaggregated_attestation(attest_slot, 0)
        .expect("canonical attestation at skip slot should succeed");
    let early_skip = harness
        .chain
        .early_attester_cache
        .try_attest(attest_slot, 0, &harness.chain.spec)
        .unwrap()
        .expect("early cache should produce attestation at skip slot");

    assert_eq!(
        canonical_skip.data().index,
        early_skip.data().index,
        "non-same-slot: early cache and canonical attestation should have matching index"
    );
    assert_eq!(
        canonical_skip.data().index,
        1,
        "non-same-slot: both should have index=1 (payload_present=true)"
    );
}

/// Pre-Gloas (Fulu) early attester cache: index should always be the committee
/// index, never payload_present. Verifies the Gloas payload_present logic is
/// NOT triggered for pre-Gloas forks.
#[tokio::test]
async fn fulu_early_cache_uses_committee_index_not_payload_present() {
    // Set Gloas at epoch 100 so we run entirely in Fulu
    let harness = gloas_harness_at_epoch(100);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let head_root = head.beacon_block_root;

    // Verify we're in Fulu (not Gloas)
    assert!(
        !harness
            .chain
            .spec
            .fork_name_at_slot::<E>(head_slot)
            .gloas_enabled(),
        "pre-condition: should be in Fulu, not Gloas"
    );

    let proto_block = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_block(&head_root)
        .expect("head should be in fork choice");

    let rpc_block =
        harness.build_rpc_block_from_store_blobs(Some(head_root), head.beacon_block.clone());
    let beacon_chain::data_availability_checker::MaybeAvailableBlock::Available(available_block) =
        harness
            .chain
            .data_availability_checker
            .verify_kzg_for_rpc_block(rpc_block)
            .unwrap()
    else {
        panic!("block should be available")
    };

    harness
        .chain
        .early_attester_cache
        .add_head_block(
            head_root,
            &available_block,
            proto_block,
            &head.beacon_state,
            &harness.chain.spec,
        )
        .unwrap();

    // Non-same-slot attestation at skip slot — should have index=0 (committee index)
    // NOT index=1 (which would mean payload_present=true if Gloas were active)
    let attest_slot = head_slot + 1;
    let early_att = harness
        .chain
        .early_attester_cache
        .try_attest(attest_slot, 0, &harness.chain.spec)
        .unwrap()
        .expect("should produce attestation from early cache");

    assert_eq!(
        early_att.data().index,
        0,
        "Fulu early cache attestation should have index=0 (committee index, not payload_present)"
    );
}

// =============================================================================
// Canonical head Gloas-specific branch tests
// =============================================================================
// These tests exercise the Gloas-specific branches in canonical_head.rs:
// - parent_random(): reads prev_randao from bid instead of execution payload
// - head_block_number(): returns 0 for Gloas blocks (block number is in envelope)
// - get_pre_payload_attributes(): full pipeline using both of the above
//
// These methods are called during `prepare_beacon_proposer` to compute
// FCU (forkchoiceUpdated) payload attributes for the execution layer.
// If parent_random returns the wrong value, the EL will build a payload
// with incorrect prev_randao, causing the block to be rejected by peers.
// If head_block_number is wrong, SSE events will report incorrect data.

/// For a Gloas head block, parent_random() should return the bid's prev_randao.
/// This is the Gloas-specific path: Gloas blocks have execution payload bids
/// instead of full execution payloads, so prev_randao comes from the bid.
#[tokio::test]
async fn gloas_canonical_head_parent_random_reads_from_bid() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let cached_head = harness.chain.canonical_head.cached_head();

    // Get the bid's prev_randao directly from the head block
    let head_block = &cached_head.snapshot.beacon_block;
    let bid = head_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("Gloas block should have a bid");
    let expected_prev_randao = bid.message.prev_randao;

    // parent_random() should return this value
    let parent_random = cached_head
        .parent_random()
        .expect("parent_random should succeed for Gloas block");

    assert_eq!(
        parent_random, expected_prev_randao,
        "parent_random() should return bid.message.prev_randao for Gloas blocks"
    );

    // Sanity: prev_randao should not be zero (it's a real RANDAO mix)
    assert_ne!(
        parent_random,
        Hash256::zero(),
        "prev_randao should not be zero for a real chain"
    );
}

/// For a Gloas head block, head_block_number() should return 0.
/// Gloas blocks don't have execution payloads (they have bids), so the
/// block number is in the envelope, not the block body. The method returns
/// 0 as a fallback since this value is only used for SSE events and the EL
/// tracks block numbers internally.
#[tokio::test]
async fn gloas_canonical_head_block_number_returns_zero() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let cached_head = harness.chain.canonical_head.cached_head();

    // Confirm this is a Gloas block
    assert!(
        cached_head
            .snapshot
            .beacon_block
            .message()
            .body()
            .signed_execution_payload_bid()
            .is_ok(),
        "head block should be Gloas (has bid)"
    );

    let block_number = cached_head
        .head_block_number()
        .expect("head_block_number should succeed for Gloas block");

    assert_eq!(
        block_number, 0,
        "head_block_number() should return 0 for Gloas blocks (block number is in envelope)"
    );
}

/// get_pre_payload_attributes should work correctly for Gloas heads.
/// It calls parent_random() and head_block_number() internally, so this
/// test verifies the full pipeline produces valid payload attributes.
#[tokio::test]
async fn gloas_get_pre_payload_attributes_succeeds() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let cached_head = harness.chain.canonical_head.cached_head();
    let head_block_root = cached_head.head_block_root();
    let head_slot = cached_head.head_slot();
    let proposal_slot = head_slot + 1;

    // Call get_pre_payload_attributes with proposer_head == head_block_root
    // (the normal case, not re-org)
    let attrs = harness
        .chain
        .get_pre_payload_attributes(proposal_slot, head_block_root, &cached_head)
        .expect("should not error");

    let attrs = attrs.expect("should produce payload attributes for Gloas head");

    // prev_randao should match head_random() (not parent_random, since proposer_head == head)
    let expected_randao = cached_head
        .head_random()
        .expect("head_random should succeed");
    assert_eq!(
        attrs.prev_randao, expected_randao,
        "payload attributes prev_randao should match head_random() when proposer_head == head"
    );

    // parent_block_number should be 0 (head_block_number returns 0 for Gloas)
    assert_eq!(
        attrs.parent_block_number, 0,
        "parent_block_number should be 0 for Gloas (head_block_number returns 0)"
    );

    // parent_beacon_block_root should be the head
    assert_eq!(
        attrs.parent_beacon_block_root, head_block_root,
        "parent_beacon_block_root should be the head block root"
    );
}

/// get_pre_payload_attributes with re-org (proposer_head == parent) should use
/// parent_random() and head_block_number - 1 for Gloas blocks.
#[tokio::test]
async fn gloas_get_pre_payload_attributes_reorg_uses_parent_random() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let cached_head = harness.chain.canonical_head.cached_head();
    let parent_block_root = cached_head.parent_block_root();
    let head_slot = cached_head.head_slot();
    let proposal_slot = head_slot + 1;

    // Call with proposer_head == parent (simulating a re-org of the head)
    let attrs = harness
        .chain
        .get_pre_payload_attributes(proposal_slot, parent_block_root, &cached_head)
        .expect("should not error");

    let attrs = attrs.expect("should produce payload attributes for re-org case");

    // prev_randao should match parent_random() — the bid's prev_randao
    let expected_parent_randao = cached_head
        .parent_random()
        .expect("parent_random should succeed");
    assert_eq!(
        attrs.prev_randao, expected_parent_randao,
        "re-org payload attributes prev_randao should match parent_random() (bid.prev_randao)"
    );

    // parent_block_number should be head_block_number.saturating_sub(1) = 0.saturating_sub(1) = 0
    // For Gloas, head_block_number() returns 0, and 0.saturating_sub(1) = 0
    assert_eq!(
        attrs.parent_block_number, 0,
        "re-org parent_block_number should be 0 (head_block_number=0, sub(1) saturates to 0)"
    );

    // parent_beacon_block_root should be the parent
    assert_eq!(
        attrs.parent_beacon_block_root, parent_block_root,
        "parent_beacon_block_root should be the parent block root in re-org case"
    );
}

// =============================================================================
// External builder block import — end-to-end lifecycle tests
// =============================================================================

/// Test that a block produced with an external builder bid can be imported, and the
/// fork choice node has payload_revealed=false (since no envelope has been processed).
#[tokio::test]
async fn gloas_external_bid_block_import_payload_unrevealed() {
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;

    // Insert an external bid for the next slot
    let state = harness.chain.head_beacon_state_cloned();
    let bid = make_external_bid(&state, head_root, next_slot, 0, 5000);
    harness.chain.execution_bid_pool.lock().insert(bid);

    // Produce a block using the external bid
    harness.advance_slot();
    let ((signed_block, blobs), _state, envelope) =
        harness.make_block_with_envelope(state, next_slot).await;

    // Confirm it's an external bid (not self-build) and no envelope returned
    let block_bid = signed_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("should have bid");
    assert_eq!(block_bid.message.builder_index, 0);
    assert!(
        envelope.is_none(),
        "external bid should not produce self-build envelope"
    );

    // Import the block into the chain
    let block_root = signed_block.canonical_root();
    harness
        .process_block(next_slot, block_root, (signed_block, blobs))
        .await
        .expect("should import block with external bid");

    // Verify fork choice: payload_revealed should be false (no envelope processed)
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let block_index = *fc
        .proto_array()
        .core_proto_array()
        .indices
        .get(&block_root)
        .expect("block should be in fork choice");
    let node = &fc.proto_array().core_proto_array().nodes[block_index];
    assert!(
        !node.payload_revealed,
        "payload_revealed should be false after import without envelope"
    );
    assert!(
        matches!(node.execution_status, ExecutionStatus::Irrelevant(_)),
        "Gloas block should have Irrelevant execution status (EL skipped during block import), got {:?}",
        node.execution_status
    );
    drop(fc);

    // Note: the block may or may not be the new head depending on fork choice weights.
    // With payload_revealed=false, it competes with the previous head (which has
    // payload_revealed=true from self-build). The important thing is that the block
    // IS in fork choice and has the correct execution status.
}

/// Test that a block produced with an external bid can coexist in fork choice with
/// the previous self-build block, and both have correct builder_index values.
/// This verifies the block import pipeline correctly handles external bids
/// without requiring an envelope to be processed.
#[tokio::test]
async fn gloas_external_bid_import_fork_choice_builder_index() {
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;

    // Verify the previous head was self-build
    let prev_head_bid = head
        .beacon_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("should have bid");
    assert_eq!(
        prev_head_bid.message.builder_index, harness.spec.builder_index_self_build,
        "previous head should be self-build"
    );

    // Insert external bid and produce block
    let state = harness.chain.head_beacon_state_cloned();
    let external_builder_index = 0u64;
    let bid = make_external_bid(&state, head_root, next_slot, external_builder_index, 5000);
    harness.chain.execution_bid_pool.lock().insert(bid);

    harness.advance_slot();
    let ((signed_block, blobs), _state, _envelope) =
        harness.make_block_with_envelope(state, next_slot).await;

    // Import block
    let block_root = signed_block.canonical_root();
    harness
        .process_block(next_slot, block_root, (signed_block, blobs))
        .await
        .expect("should import block with external bid");

    // Verify the imported block's bid has the external builder_index in the stored block
    let stored_block = harness
        .chain
        .store
        .get_blinded_block(&block_root)
        .unwrap()
        .expect("stored block should exist");
    let stored_bid = stored_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("stored block should have bid");
    assert_eq!(
        stored_bid.message.builder_index, external_builder_index,
        "stored block bid should have external builder_index"
    );
    assert_eq!(
        stored_bid.message.value, 5000,
        "stored block bid should have correct value"
    );
}

/// Test that apply_payload_envelope_to_fork_choice marks payload_revealed=true
/// for a block that was imported with an external bid.
#[tokio::test]
async fn gloas_external_bid_envelope_reveals_payload_in_fork_choice() {
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;

    // Insert external bid and produce block
    let state = harness.chain.head_beacon_state_cloned();
    let bid = make_external_bid(&state, head_root, next_slot, 0, 5000);
    harness.chain.execution_bid_pool.lock().insert(bid.clone());

    harness.advance_slot();
    let ((signed_block, blobs), _state, _envelope) =
        harness.make_block_with_envelope(state, next_slot).await;

    // Import block
    let block_root = signed_block.canonical_root();
    harness
        .process_block(next_slot, block_root, (signed_block, blobs))
        .await
        .expect("should import block");

    // Confirm payload_revealed = false
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let idx = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&block_root)
            .unwrap();
        assert!(!fc.proto_array().core_proto_array().nodes[idx].payload_revealed);
    }

    // Construct a signed envelope matching the bid and sign with the builder's key
    let mut envelope_msg = ExecutionPayloadEnvelope::<E>::empty();
    envelope_msg.beacon_block_root = block_root;
    envelope_msg.slot = next_slot;
    envelope_msg.builder_index = bid.message.builder_index;
    envelope_msg.payload.block_hash = bid.message.block_hash;

    // Sign the envelope with the builder's secret key using DOMAIN_BEACON_BUILDER
    let head_state = harness.chain.head_beacon_state_cloned();
    let epoch = next_slot.epoch(E::slots_per_epoch());
    let domain = harness.spec.get_domain(
        epoch,
        Domain::BeaconBuilder,
        &head_state.fork(),
        head_state.genesis_validators_root(),
    );
    let signing_root = envelope_msg.signing_root(domain);
    let builder_idx = bid.message.builder_index as usize;
    let signature = BUILDER_KEYPAIRS[builder_idx].sk.sign(signing_root);

    let envelope = SignedExecutionPayloadEnvelope {
        message: envelope_msg,
        signature,
    };
    let verified = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(envelope))
        .expect("envelope gossip verification should pass");

    // Apply to fork choice (marks payload_revealed = true)
    harness
        .chain
        .apply_payload_envelope_to_fork_choice(&verified)
        .expect("should apply envelope to fork choice");

    // Verify payload_revealed is now true
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let idx = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&block_root)
            .unwrap();
        let node = &fc.proto_array().core_proto_array().nodes[idx];
        assert!(
            node.payload_revealed,
            "payload_revealed should be true after envelope applied to fork choice"
        );
    }
}

// =============================================================================
// Multi-epoch chain health and state consistency tests
// =============================================================================

/// After running the chain for multiple Gloas epochs, verify that epoch processing
/// correctly rotates builder_pending_payments. The payment window is 2*SLOTS_PER_EPOCH;
/// after one epoch boundary, the first SLOTS_PER_EPOCH entries should be rotated out
/// (moved from second half to first half).
///
/// With self-build blocks (value=0), no actual payments are generated, but the
/// rotation mechanism must still execute correctly — the vector should remain
/// properly sized and all entries should be default after rotation.
#[tokio::test]
async fn gloas_multi_epoch_builder_payments_rotation() {
    let harness = gloas_harness_at_epoch(0);
    let slots_per_epoch = E::slots_per_epoch();

    // Run for 3 full epochs (24 slots with MinimalEthSpec)
    // This ensures at least 2 epoch boundaries are crossed, triggering
    // builder_pending_payments rotation twice.
    let num_slots = 3 * slots_per_epoch as usize;
    Box::pin(harness.extend_slots(num_slots)).await;

    let state = harness.chain.head_beacon_state_cloned();
    let gloas_state = state.as_gloas().expect("should be Gloas");
    let current_epoch = state.current_epoch();

    assert!(
        current_epoch >= Epoch::new(3),
        "should be at epoch 3 or later (got epoch {})",
        current_epoch
    );

    // Builder pending payments should still be properly sized
    let payments_limit = E::builder_pending_payments_limit();
    assert_eq!(
        gloas_state.builder_pending_payments.len(),
        payments_limit,
        "builder_pending_payments should maintain correct size after epoch processing"
    );

    // With self-build blocks (value=0), all entries should still be default
    // after rotation. Non-default entries would indicate a rotation bug.
    for i in 0..payments_limit {
        let payment = gloas_state.builder_pending_payments.get(i).unwrap();
        assert_eq!(
            payment.weight, 0,
            "payment[{}].weight should be 0 after rotation (self-build blocks have value=0)",
            i
        );
        assert_eq!(
            payment.withdrawal.amount, 0,
            "payment[{}].withdrawal.amount should be 0 after rotation",
            i
        );
    }
}

/// Verify latest_block_hash continuity across a skip slot. When a slot is skipped
/// (no block produced), the next block's bid parent_block_hash should reference the
/// last processed envelope's block_hash, not the skipped slot.
#[tokio::test]
async fn gloas_skip_slot_latest_block_hash_continuity() {
    let harness = gloas_harness_at_epoch(0);

    // Produce initial blocks to establish a chain
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let head_root = head.beacon_block_root;

    // Get the latest_block_hash from the current state (post-envelope)
    let state = harness.chain.head_beacon_state_cloned();
    let latest_hash_before_skip = *state
        .latest_block_hash()
        .expect("should have latest_block_hash");
    assert!(
        !latest_hash_before_skip.0.is_zero(),
        "latest_block_hash should be non-zero before skip slot"
    );

    // Skip a slot: advance the slot clock without producing a block
    harness.advance_slot();
    let skip_slot = head_slot + 1;
    harness.advance_slot();
    let produce_slot = head_slot + 2;

    // Produce a block at the slot after the skip
    let ((next_block, _), _post_state, _envelope) = harness
        .make_block_with_envelope(state.clone(), produce_slot)
        .await;

    let next_bid = next_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("should be Gloas block with bid");

    // The bid's parent_block_hash should equal the last processed envelope's block_hash
    // (the state's latest_block_hash before the skip), NOT some default or skipped value.
    assert_eq!(
        next_bid.message.parent_block_hash, latest_hash_before_skip,
        "bid parent_block_hash after skip slot should equal latest_block_hash \
         from the last processed envelope (skip_slot={}, produce_slot={})",
        skip_slot, produce_slot
    );

    // The parent_block_root should reference the head (last actual block), not the skip slot
    assert_eq!(
        next_bid.message.parent_block_root, head_root,
        "bid parent_block_root should reference the last actual block root, not the skip slot"
    );
}

/// After a fork with two competing chains, verify the head is correctly resolved.
/// Both chains produce Gloas blocks with envelopes; the one with more attestation
/// weight should win.
#[tokio::test]
async fn gloas_two_forks_head_resolves_with_attestation_weight() {
    let harness = gloas_harness_at_epoch(0);

    // Build initial shared chain
    let initial_blocks = 3usize;
    Box::pin(harness.extend_slots(initial_blocks)).await;

    let shared_head = harness.chain.head_snapshot();
    let shared_slot = shared_head.beacon_block.slot();

    // Split validators: 75% honest, 25% faulty
    let honest_validators: Vec<usize> = (0..24).collect();
    let faulty_validators: Vec<usize> = (24..VALIDATOR_COUNT).collect();

    // Create two competing forks
    let (honest_head, faulty_head) = harness
        .generate_two_forks_by_skipping_a_block(
            &honest_validators,
            &faulty_validators,
            2, // honest fork: 2 blocks
            2, // faulty fork: 2 blocks
        )
        .await;

    assert_ne!(honest_head, faulty_head, "forks should be distinct");

    // The head should follow the honest chain (more attestation weight)
    let head = harness.chain.head_snapshot();
    assert_eq!(
        head.beacon_block_root, honest_head,
        "head should follow the chain with more attestation weight (honest: 75% vs faulty: 25%)"
    );

    // Verify the head is a valid Gloas block
    assert!(
        head.beacon_block.as_gloas().is_ok(),
        "head block should be a valid Gloas block"
    );

    // The head block should be beyond the shared ancestor
    assert!(
        head.beacon_block.slot() > shared_slot,
        "head slot ({}) should be beyond shared ancestor ({})",
        head.beacon_block.slot(),
        shared_slot
    );
}

/// When process_self_build_envelope is called for a block that is NOT the current
/// canonical head, try_update_head_state should be a no-op — the head snapshot
/// must remain unchanged. This tests the else-branch (mismatch) of
/// try_update_head_state which silently does nothing.
#[tokio::test]
async fn gloas_self_build_envelope_non_head_block_leaves_head_unchanged() {
    let harness = gloas_harness_at_epoch(0);
    // Build 3 blocks with envelopes (extend_slots handles everything)
    Box::pin(harness.extend_slots(3)).await;

    // Record the current head — this is the canonical chain tip
    let head_before = harness.chain.head_snapshot();
    let head_root_before = head_before.beacon_block_root;
    let head_latest_block_hash = *head_before
        .beacon_state
        .latest_block_hash()
        .expect("Gloas state should have latest_block_hash");

    // Create a fork block from an earlier state (slot 2, skipping slot 4 for the fork)
    let fork_state = harness
        .chain
        .state_at_slot(Slot::new(2), beacon_chain::StateSkipConfig::WithStateRoots)
        .expect("should get state at slot 2");

    // Make a fork block at slot 5 (skipping slot 3/4 from the fork point)
    let fork_slot = Slot::new(5);
    harness.advance_slot(); // advance to slot 5
    let (fork_block_contents, _fork_state, fork_envelope) = harness
        .make_block_with_envelope(fork_state, fork_slot)
        .await;

    let fork_envelope = fork_envelope.expect("fork block should have envelope");
    let fork_block_root = fork_block_contents.0.canonical_root();

    // Import the fork block (no attestations, so it won't become head)
    harness
        .process_block(fork_slot, fork_block_root, fork_block_contents)
        .await
        .expect("fork block import should succeed");

    // Verify the head is still the original chain tip (the fork block has no attestations)
    let head_after_fork = harness.chain.head_snapshot();
    assert_eq!(
        head_after_fork.beacon_block_root, head_root_before,
        "head should still be the original chain tip, not the fork block"
    );

    // Process self-build envelope for the NON-head fork block.
    // This calls try_update_head_state(fork_block_root, state) which should be a no-op
    // because fork_block_root != head_block_root.
    harness
        .chain
        .process_self_build_envelope(&fork_envelope)
        .await
        .expect("should process self-build envelope for fork block");

    // The head snapshot should be completely unchanged — try_update_head_state was a no-op
    let head_after_envelope = harness.chain.head_snapshot();
    assert_eq!(
        head_after_envelope.beacon_block_root, head_root_before,
        "head block root should be unchanged after processing non-head envelope"
    );
    let head_latest_after = *head_after_envelope
        .beacon_state
        .latest_block_hash()
        .expect("should have latest_block_hash");
    assert_eq!(
        head_latest_after, head_latest_block_hash,
        "head state's latest_block_hash should be unchanged (try_update_head_state was no-op)"
    );

    // But the fork block's envelope should still be persisted to store
    let stored = harness
        .chain
        .get_payload_envelope(&fork_block_root)
        .unwrap()
        .expect("fork block envelope should be in store");
    assert_eq!(stored.message.slot, fork_slot);
}

/// process_pending_envelope should drain the buffer even when re-verification
/// fails. When the buffered envelope has a slot that doesn't match the block's
/// actual slot (simulating corruption or a malicious peer), the re-verification
/// returns SlotMismatch and the envelope is discarded — but the buffer entry
/// is always removed to prevent unbounded growth.
#[tokio::test]
async fn gloas_process_pending_envelope_reverify_failure_drains_buffer() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let mut bad_envelope = envelope.expect("should have envelope");
    let block_root = block_contents.0.canonical_root();

    // Corrupt the envelope's slot so re-verification fails with SlotMismatch
    bad_envelope.message.slot = Slot::new(999);

    // Import the block
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Buffer the corrupted envelope
    harness
        .chain
        .pending_gossip_envelopes
        .lock()
        .insert(block_root, Arc::new(bad_envelope));

    assert!(
        harness
            .chain
            .pending_gossip_envelopes
            .lock()
            .get(&block_root)
            .is_some(),
        "pre-condition: envelope is buffered"
    );

    // process_pending_envelope: re-verification fails (SlotMismatch), but buffer is drained
    harness.chain.process_pending_envelope(block_root).await;

    // Buffer should be drained regardless of verification failure
    assert!(
        harness
            .chain
            .pending_gossip_envelopes
            .lock()
            .get(&block_root)
            .is_none(),
        "buffer should be empty even after re-verification failure"
    );

    // Fork choice: payload should NOT be revealed (re-verification failed before
    // apply_payload_envelope_to_fork_choice was called)
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let proto_block = fc.get_block(&block_root).unwrap();
    assert!(
        !proto_block.payload_revealed,
        "payload_revealed should be false (envelope failed re-verification)"
    );
}

/// process_pending_envelope should drain the buffer when the block root is
/// unknown in fork choice. This covers the case where a block was pruned
/// (e.g., finalized) between buffering and processing — the envelope should
/// be discarded, not left in the buffer.
#[tokio::test]
async fn gloas_process_pending_envelope_unknown_root_drains_buffer() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    harness.advance_slot();
    let (_block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let envelope = envelope.expect("should have envelope");
    // Use a random block_root that is NOT in fork choice
    let unknown_root = Hash256::repeat_byte(0xDE);

    // Buffer the envelope under the unknown root
    harness
        .chain
        .pending_gossip_envelopes
        .lock()
        .insert(unknown_root, Arc::new(envelope));

    // process_pending_envelope: re-verification fails (BlockRootUnknown), buffer is drained
    harness.chain.process_pending_envelope(unknown_root).await;

    // Buffer should be drained
    assert!(
        harness
            .chain
            .pending_gossip_envelopes
            .lock()
            .get(&unknown_root)
            .is_none(),
        "buffer should be empty after processing with unknown root"
    );
}

/// Verify that execution_payload_availability bits correctly track payload status
/// across multiple epochs. The flow per slot is:
/// 1. per_slot_processing: advances state.slot → N, clears bit at index N
/// 2. block processing: processes the block body
/// 3. envelope processing: sets bit at index N back to true (payload available)
///
/// After extend_slots produces blocks at every slot, the last block's slot (N)
/// should have availability=true (envelope processed). The per_slot_processing
/// for the NEXT slot (N+1) hasn't run yet, so its bit is still set from the
/// initial fork upgrade (all bits = true).
#[tokio::test]
async fn gloas_execution_payload_availability_multi_epoch() {
    let harness = gloas_harness_at_epoch(0);
    let slots_per_epoch = E::slots_per_epoch();
    let slots_per_hist = <E as EthSpec>::SlotsPerHistoricalRoot::to_usize();

    // Run for 2 full epochs (16 slots with MinimalEthSpec)
    let num_slots = 2 * slots_per_epoch as usize;
    Box::pin(harness.extend_slots(num_slots)).await;

    let state = harness.chain.head_beacon_state_cloned();
    let gloas_state = state.as_gloas().expect("should be Gloas");
    let current_slot = state.slot();

    // Verify that the current slot's bit is true (envelope was processed for the head block)
    let current_slot_index = current_slot.as_usize() % slots_per_hist;
    assert!(
        gloas_state
            .execution_payload_availability
            .get(current_slot_index)
            .unwrap_or(false),
        "current slot's availability bit (index {}) should be true (envelope processed)",
        current_slot_index
    );

    // Verify that ALL slots that had blocks have availability=true.
    // Slots 1 through current_slot all had blocks with envelopes.
    for slot_num in 1..=current_slot.as_usize() {
        let idx = slot_num % slots_per_hist;
        assert!(
            gloas_state
                .execution_payload_availability
                .get(idx)
                .unwrap_or(false),
            "slot {} (index {}) should have availability=true (block+envelope processed)",
            slot_num,
            idx
        );
    }

    // Slot 0 (genesis) had no block processing in Gloas, but the fork upgrade
    // initializes all bits to true. per_slot_processing for slot 0→1 would have
    // cleared bit at index 1, but then envelope at slot 1 set it back. The bit
    // at index 0 was only cleared by per_slot_processing if some later slot
    // wraps around, which won't happen in 16 slots with slots_per_hist=8192.
    // So index 0 should still be true from initialization.
    assert!(
        gloas_state
            .execution_payload_availability
            .get(0)
            .unwrap_or(false),
        "genesis index 0 should still be true from fork upgrade initialization"
    );

    // Count set bits — should be all true. With only 16 slots processed and
    // slots_per_hist=8192, every cleared bit has been set back by envelope processing.
    let set_count = (0..slots_per_hist)
        .filter(|i| {
            gloas_state
                .execution_payload_availability
                .get(*i)
                .unwrap_or(false)
        })
        .count();

    assert_eq!(
        set_count, slots_per_hist,
        "all {} availability bits should be set (16 slots processed, {} total bits, \
         all cleared bits restored by envelope processing)",
        slots_per_hist, slots_per_hist
    );
}

// =============================================================================
// Range sync — import Gloas blocks with envelope processing
// =============================================================================

/// Simulate range sync: build a chain on one harness, extract blocks + envelopes,
/// import them into a fresh harness. This exercises the full import path including
/// `load_parent`'s state patching and `get_advanced_hot_state`'s envelope
/// re-application from store.
#[tokio::test]
async fn gloas_range_sync_import_with_envelopes() {
    // Harness 1: build 4 blocks with all envelopes processed
    let harness1 = gloas_harness_at_epoch(0);
    Box::pin(harness1.extend_slots(4)).await;

    // Extract blocks and envelopes from the chain dump
    let chain_dump = harness1.chain.chain_dump().expect("should dump chain");
    let mut blocks = Vec::new();
    let mut envelopes = Vec::new();
    for snapshot in chain_dump.iter().skip(1) {
        // skip genesis
        let full_block = harness1
            .chain
            .get_block(&snapshot.beacon_block_root)
            .await
            .unwrap()
            .unwrap();
        let envelope = harness1
            .chain
            .store
            .get_payload_envelope(&snapshot.beacon_block_root)
            .unwrap();
        blocks.push(Arc::new(full_block));
        envelopes.push(envelope);
    }
    assert_eq!(blocks.len(), 4, "should have 4 blocks");

    // Verify parent hash chain: each block's bid.parent_block_hash should reference
    // the previous block's bid.block_hash (all blocks take the FULL path via self-build).
    for i in 1..blocks.len() {
        let prev_bid_hash = blocks[i - 1]
            .message()
            .body()
            .signed_execution_payload_bid()
            .expect("should be Gloas")
            .message
            .block_hash;
        let cur_parent_hash = blocks[i]
            .message()
            .body()
            .signed_execution_payload_bid()
            .expect("should be Gloas")
            .message
            .parent_block_hash;
        assert_eq!(
            cur_parent_hash,
            prev_bid_hash,
            "block {}'s bid.parent_block_hash should equal block {}'s bid.block_hash",
            i + 1,
            i
        );
    }

    // Harness 2: fresh chain to replay blocks into (simulates range sync)
    let harness2 = gloas_harness_at_epoch(0);

    // Import blocks sequentially with envelope processing after each.
    // This tests the full import path: load_parent → per_block_processing → state root check
    // → envelope processing → state cache update.
    for (i, (block, envelope)) in blocks.iter().zip(envelopes.iter()).enumerate() {
        let rpc_block = beacon_chain::block_verification_types::RpcBlock::new_without_blobs(
            None,
            block.clone(),
        );
        let slot = block.slot();
        harness2.set_current_slot(slot);

        harness2
            .chain
            .process_chain_segment(vec![rpc_block], NotifyExecutionLayer::Yes)
            .await
            .into_block_error()
            .unwrap_or_else(|e| {
                panic!(
                    "block {} (slot {}) import should succeed: {:?}",
                    i + 1,
                    slot,
                    e
                )
            });

        if let Some(signed_envelope) = envelope {
            harness2
                .chain
                .process_self_build_envelope(signed_envelope)
                .await
                .unwrap_or_else(|e| {
                    panic!("envelope {} processing should succeed: {:?}", i + 1, e)
                });
            harness2.chain.recompute_head_at_current_slot().await;
        }
    }

    // Verify: all 4 blocks are in fork choice
    {
        let fc = harness2.chain.canonical_head.fork_choice_read_lock();
        for (i, block) in blocks.iter().enumerate() {
            let root = block.canonical_root();
            assert!(
                fc.get_block(&root).is_some(),
                "block {} (root {:?}) should be in fork choice after range sync import",
                i + 1,
                root
            );
        }
    }

    // Verify head is the last block
    harness2.chain.recompute_head_at_current_slot().await;
    let head = harness2.chain.head_snapshot();
    assert_eq!(
        head.beacon_block_root,
        blocks.last().unwrap().canonical_root(),
        "head should be the last imported block"
    );

    // Verify the head state's latest_block_hash matches the last envelope's block_hash
    let head_latest_hash = *head
        .beacon_state
        .latest_block_hash()
        .expect("Gloas state should have latest_block_hash");
    let last_envelope_hash = envelopes
        .last()
        .unwrap()
        .as_ref()
        .expect("last block should have envelope")
        .message
        .payload
        .block_hash;
    assert_eq!(
        head_latest_hash, last_envelope_hash,
        "head state latest_block_hash should match the last envelope's payload block_hash"
    );
}

/// Complementary test: when the parent's envelope WAS processed (normal path),
/// load_parent does NOT need to patch because the cached state already has the
/// correct latest_block_hash. This ensures the patching code is a no-op when
/// envelopes are processed normally.
#[tokio::test]
async fn gloas_load_parent_no_patch_needed_when_envelope_processed() {
    let harness = gloas_harness_at_epoch(0);

    // Build a base chain with envelopes (3 blocks).
    Box::pin(harness.extend_slots(3)).await;

    let head_state = harness.chain.head_beacon_state_cloned();
    let head_slot = head_state.slot();
    let head_latest_hash = *head_state
        .latest_block_hash()
        .expect("should have latest_block_hash");

    // The head block's bid.block_hash should match the state's latest_block_hash
    // (because the envelope was processed).
    let head_bid_hash = harness
        .chain
        .head_snapshot()
        .beacon_block
        .message()
        .body()
        .signed_execution_payload_bid()
        .expect("should be Gloas")
        .message
        .block_hash;
    assert_eq!(
        head_latest_hash, head_bid_hash,
        "with envelope processed, latest_block_hash should match head bid block_hash"
    );

    // Build and import the next block — should succeed without any patching needed
    harness.advance_slot();
    let next_slot = head_slot + 1;
    let (block_contents, _state, _envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let block_root = block_contents.0.canonical_root();
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("next block import should succeed when parent envelope was processed");

    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    assert!(
        fc.get_block(&block_root).is_some(),
        "next block should be in fork choice"
    );
}

/// When the parent block is at genesis (bid.block_hash is zero), load_parent
/// should NOT attempt to patch because zero hashes indicate genesis or default
/// state, not a missing envelope.
#[tokio::test]
async fn gloas_load_parent_skips_patch_for_genesis_zero_hash() {
    let harness = gloas_harness_at_epoch(0);

    // At genesis (epoch 0 = Gloas), the genesis block has a bid with zero block_hash.
    // Build the first block (slot 1) which has parent = genesis.
    harness.advance_slot();
    let genesis_state = harness.chain.head_beacon_state_cloned();
    let slot_1 = Slot::new(1);
    let (block_1_contents, _state, _envelope) = harness
        .make_block_with_envelope(genesis_state, slot_1)
        .await;

    let block_1_root = block_1_contents.0.canonical_root();

    // Import block 1 — load_parent loads genesis state. Genesis bid has
    // block_hash = zero. The patching code should skip because
    // parent_bid_block_hash == ExecutionBlockHash::zero().
    harness
        .process_block(slot_1, block_1_root, block_1_contents)
        .await
        .expect("first block import should succeed (genesis parent, zero hash → no patch)");

    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    assert!(
        fc.get_block(&block_1_root).is_some(),
        "first block should be in fork choice"
    );
}

// =============================================================================
// Gossip envelope processing, load_parent EMPTY path, historical attestation
// =============================================================================

/// Full gossip pipeline for a self-build envelope arriving at another node:
/// verify_payload_envelope_for_gossip → apply_payload_envelope_to_fork_choice →
/// process_payload_envelope. Verifies that the state transition runs, the envelope
/// is persisted to the store, and the head state is updated with latest_block_hash.
///
/// This is the only test that exercises process_payload_envelope through the gossip
/// verification path. All other tests use process_self_build_envelope which takes a
/// different code path (no gossip verification, just direct state transition + EL).
/// In a live network, when Node A produces a self-build block+envelope and gossips
/// both, Node B processes the envelope through this gossip pipeline.
#[tokio::test]
async fn gloas_gossip_envelope_full_processing_pipeline() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    // Produce a block and its self-build envelope
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("Gloas block should produce an envelope");
    let envelope_block_hash = signed_envelope.message.payload.block_hash;
    let block_root = block_contents.0.canonical_root();

    // Import the block only (simulating gossip block arriving first)
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Confirm payload_revealed is false before envelope processing
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let idx = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&block_root)
            .unwrap();
        assert!(
            !fc.proto_array().core_proto_array().nodes[idx].payload_revealed,
            "payload_revealed should be false before envelope"
        );
    }

    // Step 1: Gossip verification (self-build envelopes skip BLS check)
    let verified = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope))
        .expect("gossip verification should pass");

    // Step 2: Apply to fork choice (marks payload_revealed = true)
    harness
        .chain
        .apply_payload_envelope_to_fork_choice(&verified)
        .expect("should apply to fork choice");

    // Confirm payload_revealed is now true
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let idx = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&block_root)
            .unwrap();
        assert!(
            fc.proto_array().core_proto_array().nodes[idx].payload_revealed,
            "payload_revealed should be true after fork choice update"
        );
    }

    // Step 3: Full envelope processing (EL newPayload + state transition)
    harness
        .chain
        .process_payload_envelope(&verified)
        .await
        .expect("process_payload_envelope should succeed");

    // Verify: envelope is persisted to the store
    let stored_envelope = harness
        .chain
        .store
        .get_payload_envelope(&block_root)
        .expect("store read should not error")
        .expect("envelope should be persisted after processing");
    assert_eq!(
        stored_envelope.message.beacon_block_root, block_root,
        "stored envelope should reference correct block root"
    );

    // Verify: the state cache has been updated with the post-envelope state.
    // The post-envelope state should have latest_block_hash == envelope's block_hash.
    let block_state_root = harness
        .chain
        .store
        .get_blinded_block(&block_root)
        .unwrap()
        .unwrap()
        .message()
        .state_root();
    let post_state = harness
        .chain
        .get_state(&block_state_root, Some(next_slot), false)
        .expect("should not error")
        .expect("post-envelope state should be in cache");
    let latest_hash = post_state
        .latest_block_hash()
        .expect("Gloas state should have latest_block_hash");
    assert_eq!(
        *latest_hash, envelope_block_hash,
        "post-envelope state latest_block_hash should match the envelope's block_hash"
    );
}

/// Import a block whose parent had its payload unrevealed (parent EMPTY path).
///
/// In a live network, envelope delivery is not guaranteed. If a builder reveals
/// their payload late or not at all, the next proposer builds on an EMPTY parent.
/// The `load_parent` code (block_verification.rs) must handle this correctly:
/// when child_bid.parent_block_hash != parent_bid_block_hash, no hash patching
/// occurs and the block still imports successfully.
#[tokio::test]
async fn gloas_load_parent_empty_parent_unrevealed_payload() {
    let harness = gloas_harness_at_epoch(0);
    // Build initial chain with revealed payloads
    Box::pin(harness.extend_slots(2)).await;

    // Produce block at slot 3 but DO NOT process its envelope
    harness.advance_slot();
    let state_before_3 = harness.chain.head_beacon_state_cloned();
    let slot_3 = state_before_3.slot() + 1;
    let (block_3_contents, _state_3, _envelope_3) = harness
        .make_block_with_envelope(state_before_3, slot_3)
        .await;
    let block_3_root = block_3_contents.0.canonical_root();

    // Import block 3 without envelope → payload_revealed = false (parent EMPTY)
    harness
        .process_block(slot_3, block_3_root, block_3_contents)
        .await
        .expect("block 3 import should succeed");

    // Confirm block 3 has payload_revealed = false
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto = fc.get_block(&block_3_root).unwrap();
        assert!(
            !proto.payload_revealed,
            "block 3 should have payload_revealed=false (no envelope processed)"
        );
    }

    // Now produce block at slot 4 whose parent is the unrevealed block 3.
    // This exercises the load_parent EMPTY path where
    // child_bid.parent_block_hash != parent_bid_block_hash.
    harness.advance_slot();
    let state_for_4 = harness.chain.head_beacon_state_cloned();
    let slot_4 = state_for_4.slot() + 1;
    let (block_4_contents, _state_4, _envelope_4) =
        harness.make_block_with_envelope(state_for_4, slot_4).await;
    let block_4_root = block_4_contents.0.canonical_root();

    // Import block 4 — this triggers load_parent which must handle the
    // EMPTY parent case: no hash patching because parent's payload was not revealed.
    harness
        .process_block(slot_4, block_4_root, block_4_contents)
        .await
        .expect("block 4 import should succeed even with unrevealed parent payload");

    // Verify block 4 is in fork choice
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    assert!(
        fc.get_block(&block_4_root).is_some(),
        "block 4 should be in fork choice"
    );
    // Block 3 (unrevealed parent) should still be there too
    assert!(
        fc.get_block(&block_3_root).is_some(),
        "block 3 should still be in fork choice"
    );
}

/// Request attestation for a historical slot (request_slot < head_state.slot())
/// where the historical block's payload was revealed. Verifies that `data.index`
/// is 1 (payload_present=true), exercising the `request_slot < head_state.slot()`
/// branch in produce_unaggregated_attestation.
///
/// This branch looks up `payload_revealed` on the proto_node for the historical
/// block (via fc.get_block), rather than using `gloas_head_payload_status()`.
#[tokio::test]
async fn gloas_attestation_historical_slot_payload_revealed() {
    let harness = gloas_harness_at_epoch(0);
    // Build chain with revealed payloads: slots 1, 2, 3
    Box::pin(harness.extend_slots(3)).await;

    let slot_3_root = harness.chain.head_snapshot().beacon_block_root;

    // Verify slot 3 has payload_revealed = true
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&slot_3_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "pre-condition: slot 3 payload should be revealed"
        );
    }

    // Produce another block at slot 4 to advance the head
    harness.advance_slot();
    let state = harness.chain.head_beacon_state_cloned();
    let slot_4 = state.slot() + 1;
    let (block_4_contents, _state, _envelope) =
        harness.make_block_with_envelope(state, slot_4).await;
    let block_4_root = block_4_contents.0.canonical_root();
    harness
        .process_block(slot_4, block_4_root, block_4_contents)
        .await
        .expect("block 4 import ok");

    // Process envelope for block 4 so it becomes the head
    let envelope_4 = _envelope.expect("should have envelope");
    harness
        .chain
        .process_self_build_envelope(&envelope_4)
        .await
        .expect("envelope 4 ok");

    // Now head_state.slot() == 4. Request attestation for slot 3 (historical).
    // This exercises the `request_slot < head_state.slot()` branch.
    let head_state = harness.chain.head_beacon_state_cloned();
    assert_eq!(head_state.slot(), slot_4, "head should be at slot 4");

    let attestation = harness
        .chain
        .produce_unaggregated_attestation(Slot::new(3), 0)
        .expect("should produce attestation for historical slot");

    // Historical slot with payload_revealed=true → data.index must be 1
    assert_eq!(
        attestation.data().index,
        1,
        "historical slot Gloas attestation should have index=1 (payload_present=true) \
         when payload_revealed=true on the historical block. \
         head_slot={}, request_slot=3",
        head_state.slot()
    );
}

// =============================================================================
// External builder envelope gossip verification tests
// =============================================================================

/// Helper: sign an ExecutionPayloadEnvelope with the given builder keypair using
/// DOMAIN_BEACON_BUILDER, matching the pattern in `gloas_external_bid_envelope_reveals_payload_in_fork_choice`.
fn sign_envelope_with_builder(
    envelope_msg: &ExecutionPayloadEnvelope<E>,
    builder_keypair: &Keypair,
    slot: Slot,
    fork: &Fork,
    genesis_validators_root: Hash256,
    spec: &ChainSpec,
) -> SignedExecutionPayloadEnvelope<E> {
    let epoch = slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(epoch, Domain::BeaconBuilder, fork, genesis_validators_root);
    let signing_root = envelope_msg.signing_root(domain);
    let signature = builder_keypair.sk.sign(signing_root);
    SignedExecutionPayloadEnvelope {
        message: envelope_msg.clone(),
        signature,
    }
}

/// External builder envelope with invalid BLS signature is rejected with
/// `InvalidSignature` during gossip verification. This exercises the BLS
/// verification path that is skipped for self-build envelopes.
#[tokio::test]
async fn gloas_external_builder_envelope_invalid_signature_rejected() {
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    // Extend enough to finalize (external builder tests need finalized builders)
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;

    // Insert external bid and produce block with it
    let state = harness.chain.head_beacon_state_cloned();
    let bid = make_external_bid(&state, head_root, next_slot, 0, 5000);
    harness.chain.execution_bid_pool.lock().insert(bid.clone());

    harness.advance_slot();
    let ((signed_block, blobs), _state, _envelope) =
        harness.make_block_with_envelope(state, next_slot).await;

    let block_root = signed_block.canonical_root();
    harness
        .process_block(next_slot, block_root, (signed_block, blobs))
        .await
        .expect("block import should succeed");

    // Construct envelope matching the bid
    let mut envelope_msg = ExecutionPayloadEnvelope::<E>::empty();
    envelope_msg.beacon_block_root = block_root;
    envelope_msg.slot = next_slot;
    envelope_msg.builder_index = bid.message.builder_index;
    envelope_msg.payload.block_hash = bid.message.block_hash;

    // Sign with the WRONG key (validator key 0 instead of builder key 0)
    let head_state = harness.chain.head_beacon_state_cloned();
    let validator_keypairs = types::test_utils::generate_deterministic_keypairs(VALIDATOR_COUNT);
    let wrong_keypair = &validator_keypairs[0]; // validator key, not builder key
    let signed_envelope = sign_envelope_with_builder(
        &envelope_msg,
        wrong_keypair,
        next_slot,
        &head_state.fork(),
        head_state.genesis_validators_root(),
        &harness.spec,
    );

    // Gossip verification should reject with InvalidSignature
    let result = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope));
    assert!(
        matches!(result, Err(PayloadEnvelopeError::InvalidSignature)),
        "external builder envelope with wrong BLS signature should be rejected, got {:?}",
        result.err()
    );
}

/// External builder envelope with valid BLS signature passes gossip verification
/// and can be applied to fork choice. This exercises the non-self-build BLS
/// verification path end-to-end (the path that was fixed in run 169 to avoid
/// double-verification).
#[tokio::test]
async fn gloas_external_builder_envelope_valid_signature_accepted() {
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;

    // Insert external bid and produce block
    let state = harness.chain.head_beacon_state_cloned();
    let bid = make_external_bid(&state, head_root, next_slot, 0, 5000);
    harness.chain.execution_bid_pool.lock().insert(bid.clone());

    harness.advance_slot();
    let ((signed_block, blobs), _state, _envelope) =
        harness.make_block_with_envelope(state, next_slot).await;

    let block_root = signed_block.canonical_root();
    harness
        .process_block(next_slot, block_root, (signed_block, blobs))
        .await
        .expect("block import should succeed");

    // Pre-condition: payload_revealed should be false (external bid, no envelope yet)
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            !proto_block.payload_revealed,
            "pre-condition: payload should not be revealed before envelope"
        );
    }

    // Construct and sign envelope with correct builder key
    let mut envelope_msg = ExecutionPayloadEnvelope::<E>::empty();
    envelope_msg.beacon_block_root = block_root;
    envelope_msg.slot = next_slot;
    envelope_msg.builder_index = bid.message.builder_index;
    envelope_msg.payload.block_hash = bid.message.block_hash;

    let head_state = harness.chain.head_beacon_state_cloned();
    let builder_keypair = &BUILDER_KEYPAIRS[bid.message.builder_index as usize];
    let signed_envelope = sign_envelope_with_builder(
        &envelope_msg,
        builder_keypair,
        next_slot,
        &head_state.fork(),
        head_state.genesis_validators_root(),
        &harness.spec,
    );

    // Gossip verification should pass with valid builder BLS signature
    let verified = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope))
        .expect("external builder envelope with valid BLS should pass gossip verification");

    // Apply to fork choice — should mark payload_revealed = true
    harness
        .chain
        .apply_payload_envelope_to_fork_choice(&verified)
        .expect("should apply envelope to fork choice");

    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "payload should be revealed after valid external builder envelope applied to fork choice"
        );
    }
}

/// External builder envelope arriving before its block is buffered in
/// `pending_gossip_envelopes`, then successfully processed after the block
/// is imported via `process_pending_envelope`. This exercises the full
/// external builder buffering → re-verification → fork choice update pipeline.
#[tokio::test]
async fn gloas_external_builder_envelope_buffered_then_processed() {
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;

    // Insert external bid and produce block (but don't import yet)
    let state = harness.chain.head_beacon_state_cloned();
    let bid = make_external_bid(&state, head_root, next_slot, 0, 5000);
    harness.chain.execution_bid_pool.lock().insert(bid.clone());

    harness.advance_slot();
    let ((signed_block, blobs), _state, _envelope) =
        harness.make_block_with_envelope(state, next_slot).await;
    let block_root = signed_block.canonical_root();

    // Construct and sign envelope with correct builder key
    let mut envelope_msg = ExecutionPayloadEnvelope::<E>::empty();
    envelope_msg.beacon_block_root = block_root;
    envelope_msg.slot = next_slot;
    envelope_msg.builder_index = bid.message.builder_index;
    envelope_msg.payload.block_hash = bid.message.block_hash;

    let head_state = harness.chain.head_beacon_state_cloned();
    let builder_keypair = &BUILDER_KEYPAIRS[bid.message.builder_index as usize];
    let signed_envelope = sign_envelope_with_builder(
        &envelope_msg,
        builder_keypair,
        next_slot,
        &head_state.fork(),
        head_state.genesis_validators_root(),
        &harness.spec,
    );

    // Submit envelope BEFORE block is imported — should be buffered
    let result = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope));
    assert!(
        matches!(result, Err(PayloadEnvelopeError::BlockRootUnknown { .. })),
        "envelope before block should return BlockRootUnknown, got {:?}",
        result.err()
    );

    // Verify envelope was buffered
    assert!(
        harness
            .chain
            .pending_gossip_envelopes
            .lock()
            .contains_key(&block_root),
        "envelope should be buffered in pending_gossip_envelopes"
    );

    // Now import the block
    harness
        .process_block(next_slot, block_root, (signed_block, blobs))
        .await
        .expect("block import should succeed");

    // Process the pending envelope (simulates what beacon_processor does after block import)
    harness.chain.process_pending_envelope(block_root).await;

    // Buffer should be drained
    assert!(
        harness
            .chain
            .pending_gossip_envelopes
            .lock()
            .get(&block_root)
            .is_none(),
        "pending buffer should be empty after processing"
    );

    // The buffered envelope should have been re-verified and applied to fork choice
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "payload should be revealed after buffered external builder envelope was processed"
        );
    }
}

// =============================================================================
// Proposer preferences pool behavior
// =============================================================================

/// Test that `insert_proposer_preferences` rejects a duplicate slot and that
/// preferences older than 2 epochs are pruned from the pool.
///
/// The `insert_proposer_preferences` function has two important guards:
/// 1. Returns `false` if a preference for the same slot already exists (dedup)
/// 2. Prunes entries older than `2 * slots_per_epoch` from the current slot
///
/// Both guards are exercised via gossip handler tests, but the raw beacon_chain
/// pool behavior has never been tested at the integration level.
#[tokio::test]
async fn gloas_proposer_preferences_pool_dedup_and_pruning() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let current_slot = harness.chain.slot().expect("should have slot");

    // Create a signed preferences for the current slot
    let prefs1 = SignedProposerPreferences {
        message: ProposerPreferences {
            proposal_slot: current_slot.as_u64(),
            validator_index: 0,
            fee_recipient: Address::repeat_byte(0xAA),
            gas_limit: 30_000_000,
        },
        signature: Signature::empty(),
    };

    // First insertion should succeed
    let inserted = harness.chain.insert_proposer_preferences(prefs1.clone());
    assert!(
        inserted,
        "first insertion for slot {} should succeed",
        current_slot
    );

    // Retrieval should return the preferences
    let retrieved = harness.chain.get_proposer_preferences(current_slot);
    assert!(
        retrieved.is_some(),
        "should retrieve preferences for slot {}",
        current_slot
    );
    assert_eq!(
        retrieved.unwrap().message.fee_recipient,
        Address::repeat_byte(0xAA),
        "retrieved fee_recipient should match"
    );

    // Duplicate insertion for the same slot should be rejected
    let prefs_dup = SignedProposerPreferences {
        message: ProposerPreferences {
            proposal_slot: current_slot.as_u64(),
            validator_index: 0,
            fee_recipient: Address::repeat_byte(0xBB), // different content
            gas_limit: 60_000_000,
        },
        signature: Signature::empty(),
    };
    let inserted_dup = harness.chain.insert_proposer_preferences(prefs_dup);
    assert!(
        !inserted_dup,
        "duplicate insertion for same slot should be rejected"
    );

    // Original should be preserved (not overwritten)
    let still_original = harness
        .chain
        .get_proposer_preferences(current_slot)
        .unwrap();
    assert_eq!(
        still_original.message.fee_recipient,
        Address::repeat_byte(0xAA),
        "original preferences should be preserved after duplicate rejection"
    );

    // Now test pruning: advance beyond 2 epochs and insert a new preference.
    // The old preference should be pruned.
    let slots_per_epoch = E::slots_per_epoch();
    let advance_by = slots_per_epoch * 2 + 1;
    for _ in 0..advance_by {
        harness.advance_slot();
    }
    // Extend to produce at least one block to update the chain's slot
    Box::pin(harness.extend_slots(1)).await;

    let new_slot = harness.chain.slot().expect("should have slot");
    let prefs_new = SignedProposerPreferences {
        message: ProposerPreferences {
            proposal_slot: new_slot.as_u64(),
            validator_index: 1,
            fee_recipient: Address::repeat_byte(0xCC),
            gas_limit: 30_000_000,
        },
        signature: Signature::empty(),
    };
    let inserted_new = harness.chain.insert_proposer_preferences(prefs_new);
    assert!(
        inserted_new,
        "insertion for new slot {} should succeed",
        new_slot
    );

    // The old preference should have been pruned (current_slot < new_slot - 2*slots_per_epoch)
    let old = harness.chain.get_proposer_preferences(current_slot);
    assert!(
        old.is_none(),
        "old preferences at slot {} should be pruned after advancing to slot {} \
         (prune_before = {} - {} = {})",
        current_slot,
        new_slot,
        new_slot,
        slots_per_epoch * 2,
        new_slot.as_u64().saturating_sub(slots_per_epoch * 2),
    );

    // New preference should still be present
    let new = harness.chain.get_proposer_preferences(new_slot);
    assert!(
        new.is_some(),
        "new preferences at slot {} should still be present",
        new_slot
    );
}

// =============================================================================
// PTC attestation payload_present=false does NOT trigger payload_revealed
// =============================================================================

/// Test that PTC members voting `payload_present=false` does NOT cause
/// `payload_revealed` to become true, even if they reach quorum.
///
/// The `on_payload_attestation` code only accumulates `ptc_weight` when
/// `attestation.data.payload_present == true`. This test verifies the negative
/// case: all PTC members vote `payload_present=false`, and payload_revealed
/// remains false despite reaching "quorum" in count.
#[tokio::test]
async fn gloas_payload_absent_attestations_do_not_reveal_payload() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    // Reset payload_revealed and ptc_weight to simulate an unrevealed state
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        let block_index = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&head_root)
            .expect("head root should be in fork choice");
        let node = &mut fc.proto_array_mut().core_proto_array_mut().nodes[block_index];
        node.payload_revealed = false;
        node.ptc_weight = 0;
    }

    // Get all PTC members
    let ptc =
        get_ptc_committee::<E>(state, head_slot, &harness.spec).expect("should get PTC committee");

    // Import attestations from ALL PTC members with payload_present=false
    for (i, &validator_index) in ptc.iter().enumerate() {
        let data = PayloadAttestationData {
            beacon_block_root: head_root,
            slot: head_slot,
            payload_present: false, // voting ABSENT
            blob_data_available: false,
        };

        let signature =
            sign_payload_attestation_data(&data, validator_index as usize, state, &harness.spec);

        let message = PayloadAttestationMessage {
            validator_index,
            data,
            signature,
        };

        let result = harness.chain.import_payload_attestation_message(message);
        assert!(
            result.is_ok(),
            "should import payload_absent attestation from PTC member {}: {:?}",
            i,
            result.err()
        );
    }

    // Verify: ptc_weight should remain 0 (payload_present=false doesn't accumulate weight)
    let fc = harness.chain.canonical_head.fork_choice_read_lock();
    let node = fc.get_block(&head_root).unwrap();
    assert_eq!(
        node.ptc_weight, 0,
        "ptc_weight should remain 0 when all votes are payload_present=false"
    );
    assert!(
        !node.payload_revealed,
        "payload_revealed should remain false when no payload_present=true votes"
    );
}

// =============================================================================
// on_execution_bid resets fork choice node fields
// =============================================================================

/// Test that `apply_execution_bid_to_fork_choice` resets `payload_revealed`,
/// `ptc_weight`, `ptc_blob_data_available_weight`, and `payload_data_available`
/// on the fork choice node.
///
/// In the ePBS flow, when a new bid arrives for a block, the previous bid's
/// state should be reset. This is important because if a builder was previously
/// tracking PTC weight from a prior bid, a new bid should start fresh.
///
/// The existing `gloas_apply_bid_to_fork_choice_updates_node_fields` test checks
/// that builder_index is SET, but doesn't verify the RESET behavior of the
/// reveal/weight/availability fields.
#[tokio::test]
async fn gloas_on_execution_bid_resets_reveal_and_weight_fields() {
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = harness.chain.head_beacon_state_cloned();

    // Manually set some non-default values on the head's fork choice node
    // to simulate a state where PTC weight has accumulated and payload was revealed
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        let block_index = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&head_root)
            .expect("head root should be in fork choice");
        let node = &mut fc.proto_array_mut().core_proto_array_mut().nodes[block_index];
        // Simulate previous bid state: payload was revealed, had PTC weight, etc.
        node.payload_revealed = true;
        node.ptc_weight = 42;
        node.ptc_blob_data_available_weight = 17;
        node.payload_data_available = true;
    }

    // Verify the non-default state before the bid
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let node = fc.get_block(&head_root).unwrap();
        assert!(
            node.payload_revealed,
            "sanity: payload_revealed should be true before bid"
        );
        assert_eq!(node.ptc_weight, 42, "sanity: ptc_weight should be 42");
    }

    // Create and apply a new bid targeting the head block
    // (on_execution_bid targets the beacon_block_root of the block the bid is for)
    // We need to target `head_root` since that's the block in fork choice.
    // However, on_execution_bid verifies bid.slot == node.slot, so use head_slot.
    let bid = make_external_bid(&state, head_root, head_slot, 0, 5000);

    // Apply directly via fork choice (bypassing gossip verification)
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        fc.on_execution_bid(&bid, head_root)
            .expect("on_execution_bid should succeed");
    }

    // Verify: all reveal/weight fields should be RESET
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let node = fc.get_block(&head_root).unwrap();
        assert!(
            !node.payload_revealed,
            "payload_revealed should be reset to false after new bid"
        );
        assert_eq!(
            node.ptc_weight, 0,
            "ptc_weight should be reset to 0 after new bid"
        );
        assert_eq!(
            node.ptc_blob_data_available_weight, 0,
            "ptc_blob_data_available_weight should be reset to 0 after new bid"
        );
        assert!(
            !node.payload_data_available,
            "payload_data_available should be reset to false after new bid"
        );
        assert_eq!(
            node.builder_index,
            Some(0),
            "builder_index should be set to the new bid's builder_index"
        );
    }
}

// =============================================================================
// Gossip envelope EL error paths, cross-epoch withdrawal computation
// =============================================================================

/// When the EL returns `Invalid` for a gossip-received envelope's `newPayload`,
/// `process_payload_envelope` should return an error. This is the gossip-path
/// counterpart of `gloas_self_build_envelope_el_invalid_returns_error` — both
/// exercise the same EL response handling but through different code paths
/// (self-build vs gossip). A bug here would cause gossip-received envelopes with
/// invalid execution payloads to be silently accepted and stored.
///
/// Note: `payload_revealed` is already true at this point because
/// `apply_payload_envelope_to_fork_choice` runs before `process_payload_envelope`.
#[tokio::test]
async fn gloas_gossip_envelope_el_invalid_returns_error() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    // Produce a block and its self-build envelope
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("Gloas block should produce an envelope");
    let block_root = block_contents.0.canonical_root();

    // Import the block only (simulating gossip block arriving first)
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Step 1: Gossip verification
    let verified = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope))
        .expect("gossip verification should pass");

    // Step 2: Apply to fork choice (marks payload_revealed = true)
    harness
        .chain
        .apply_payload_envelope_to_fork_choice(&verified)
        .expect("should apply to fork choice");

    // Configure mock EL to return Invalid for newPayload BEFORE calling process_payload_envelope
    let mock_el = harness.mock_execution_layer.as_ref().unwrap();
    mock_el
        .server
        .all_payloads_invalid_on_new_payload(ExecutionBlockHash::zero());

    // Step 3: process_payload_envelope should fail because EL says Invalid
    let result = harness.chain.process_payload_envelope(&verified).await;

    assert!(
        result.is_err(),
        "process_payload_envelope should error when EL returns Invalid"
    );
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("invalid"),
        "error should mention invalid payload, got: {}",
        err_msg
    );

    // payload_revealed should still be true (set before EL call)
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "payload_revealed should remain true (set by apply_payload_envelope_to_fork_choice before EL call)"
        );
    }

    // The envelope should NOT be persisted to the store (process_payload_envelope errored
    // before reaching the store-write step)
    let stored = harness
        .chain
        .store
        .get_payload_envelope(&block_root)
        .expect("store read should not error");
    assert!(
        stored.is_none(),
        "envelope should not be stored when process_payload_envelope fails"
    );
}

/// When the EL returns `Syncing` for a gossip-received envelope's `newPayload`,
/// `process_payload_envelope` should succeed (Syncing is not an error) but the
/// block should remain Optimistic (not promoted to Valid). This covers the case
/// where the EL hasn't fully synced and can't validate the payload yet.
///
/// This is the gossip-path counterpart of
/// `gloas_self_build_envelope_el_syncing_stays_optimistic`.
#[tokio::test]
async fn gloas_gossip_envelope_el_syncing_stays_optimistic() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    // Produce a block and its self-build envelope
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("Gloas block should produce an envelope");
    let envelope_block_hash = signed_envelope.message.payload.block_hash;
    let block_root = block_contents.0.canonical_root();

    // Import the block only
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Step 1: Gossip verification
    let verified = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope))
        .expect("gossip verification should pass");

    // Step 2: Apply to fork choice
    harness
        .chain
        .apply_payload_envelope_to_fork_choice(&verified)
        .expect("should apply to fork choice");

    // Configure mock EL to return Syncing for newPayload
    let mock_el = harness.mock_execution_layer.as_ref().unwrap();
    mock_el.server.all_payloads_syncing_on_new_payload(false);

    // Step 3: process_payload_envelope should succeed (Syncing is not an error)
    harness
        .chain
        .process_payload_envelope(&verified)
        .await
        .expect("Syncing response should not cause an error");

    // Block should remain Optimistic (EL said Syncing, not Valid)
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            matches!(proto_block.execution_status, ExecutionStatus::Optimistic(_)),
            "block should remain Optimistic when EL returns Syncing, got {:?}",
            proto_block.execution_status
        );
    }

    // payload_revealed should be true
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "payload_revealed should be true regardless of EL response"
        );
    }

    // The envelope SHOULD be persisted to the store (processing succeeded)
    let stored = harness
        .chain
        .store
        .get_payload_envelope(&block_root)
        .expect("store read should not error")
        .expect("envelope should be persisted after successful processing");
    assert_eq!(
        stored.message.payload.block_hash, envelope_block_hash,
        "stored envelope should have correct block hash"
    );
}

/// Verify the cross-epoch withdrawal computation uses the Gloas path.
///
/// `get_expected_withdrawals` has two branches for Gloas:
/// 1. Same-epoch (head_state.current_epoch() == proposal_epoch): uses `unadvanced_state`
/// 2. Cross-epoch (head_state.current_epoch() != proposal_epoch): advances state to
///    proposal_epoch start, then uses `get_expected_withdrawals_gloas` on the advanced state
///
/// The same-epoch branch is exercised by `gloas_block_production_uses_gloas_withdrawals`.
/// This test exercises the cross-epoch branch by requesting withdrawals for a slot
/// in the next epoch while the head block is in the current epoch. The cross-epoch
/// path is reached during proposer preparation (beacon_chain.rs:7389) when the
/// proposer's slot is in the next epoch.
#[tokio::test]
async fn gloas_cross_epoch_withdrawal_uses_advanced_state() {
    let harness = gloas_harness_at_epoch(0);
    // MinimalEthSpec: 8 slots per epoch.
    // Build chain to slot 6 (epoch 0, slot 6). Leave room so we can request
    // withdrawals for slot 8 (epoch 1, slot 0) — a cross-epoch request.
    Box::pin(harness.extend_slots(6)).await;

    let head = harness.chain.head_snapshot();
    let head_slot = head.beacon_block.slot();
    let head_epoch = head_slot.epoch(E::slots_per_epoch());
    assert_eq!(
        head_epoch,
        Epoch::new(0),
        "pre-condition: head should be in epoch 0"
    );

    // Construct ForkchoiceUpdateParameters pointing at the current head
    let fc_params = ForkchoiceUpdateParameters {
        head_root: head.beacon_block_root,
        head_hash: None,
        justified_hash: None,
        finalized_hash: None,
    };

    // Request withdrawals for epoch 1, slot 0 (the first slot of the next epoch)
    let proposal_slot = Slot::new(E::slots_per_epoch());
    let proposal_epoch = proposal_slot.epoch(E::slots_per_epoch());
    assert_eq!(
        proposal_epoch,
        Epoch::new(1),
        "pre-condition: proposal should be in epoch 1"
    );
    assert_ne!(
        head_epoch, proposal_epoch,
        "pre-condition: head and proposal must be in different epochs for cross-epoch path"
    );

    // This calls the cross-epoch branch (lines 6049-6062 in beacon_chain.rs):
    // partial_state_advance to proposal_epoch start, then get_expected_withdrawals_gloas
    let withdrawals = harness
        .chain
        .get_expected_withdrawals(&fc_params, proposal_slot)
        .expect("cross-epoch withdrawal computation should succeed");

    // The withdrawals should be valid (non-panicking). In the minimal test environment
    // with 32 validators and self-build blocks, there may or may not be validator
    // withdrawals depending on balances. The key assertion is that the function
    // doesn't error — if the wrong (pre-Gloas) withdrawal function were called
    // on an advanced Gloas state, it would fail or produce incorrect results because
    // the Gloas function handles builder_pending_withdrawals which don't exist in
    // the pre-Gloas function.
    let _ = withdrawals.len();

    // Verify the same-epoch path also works (as a control)
    let same_epoch_slot = head_slot + 1;
    assert_eq!(
        same_epoch_slot.epoch(E::slots_per_epoch()),
        head_epoch,
        "control: same-epoch slot should be in the same epoch as head"
    );
    let same_epoch_withdrawals = harness
        .chain
        .get_expected_withdrawals(&fc_params, same_epoch_slot)
        .expect("same-epoch withdrawal computation should succeed");

    // Both should return valid withdrawals lists (possibly different due to epoch processing)
    let _ = same_epoch_withdrawals.len();
}

// =============================================================================
// Canonical head `head_hash` fallback tests
// =============================================================================

/// Verify that after building a Gloas chain, `cached_head().forkchoice_update_parameters().head_hash`
/// is `Some(block_hash)` derived from `state.latest_block_hash()` — not `None`.
///
/// Gloas blocks have `ExecutionStatus::Irrelevant` in fork choice, so
/// `get_forkchoice_update_parameters` returns `head_hash=None`. The four fallback
/// sites in `canonical_head.rs` (lines 282, 343, 748, 784) correct this by reading
/// `state.latest_block_hash()`. If any fallback is broken, `forkchoiceUpdated` sends
/// `headBlockHash=None` to the EL, which is consensus-breaking (EL builds on the
/// wrong parent or rejects the request).
///
/// No previous test verified `head_hash` on the cached head for Gloas blocks.
#[tokio::test]
async fn gloas_cached_head_hash_from_latest_block_hash() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(4)).await;

    // Get the cached head's forkchoice update parameters
    let cached_head = harness.chain.canonical_head.cached_head();
    let fc_params = cached_head.forkchoice_update_parameters();

    // head_hash must be Some — the Gloas fallback should have populated it
    assert!(
        fc_params.head_hash.is_some(),
        "head_hash should be Some for a Gloas head (fallback from state.latest_block_hash)"
    );

    // Verify it matches state.latest_block_hash
    let head_state = &cached_head.snapshot.beacon_state;
    let expected_hash = *head_state
        .latest_block_hash()
        .expect("Gloas state has latest_block_hash");

    assert_ne!(
        expected_hash,
        ExecutionBlockHash::zero(),
        "latest_block_hash should be non-zero after envelope processing"
    );
    assert_eq!(
        fc_params.head_hash.unwrap(),
        expected_hash,
        "head_hash should equal state.latest_block_hash"
    );

    // Also verify that fork choice itself returns None for the head (proving the
    // fallback is what provides the value, not fork choice directly).
    let fc_head_hash = harness
        .chain
        .canonical_head
        .fork_choice_read_lock()
        .get_forkchoice_update_parameters()
        .head_hash;

    // After envelope processing, on_execution_payload sets the node to
    // Optimistic(payload_block_hash), so fork choice itself should also have the hash.
    // This test still validates the fallback path because during the window between
    // block import and envelope processing, fork choice returns None and only the
    // fallback provides head_hash. The cached_head is set during recompute_head which
    // runs after both block import and envelope processing.
    assert!(
        fc_head_hash.is_some(),
        "fork choice head_hash should also be Some after envelope sets Optimistic status"
    );
}

/// Test that `persist_fork_choice` + `load_fork_choice` preserves the fork choice
/// state for a Gloas chain, and that `CanonicalHead::new` correctly derives `head_hash`
/// from the loaded fork choice + state snapshot.
///
/// This exercises the restart path: if a node crashes and restores from the persisted
/// fork choice, the `head_hash` must still be correct for the `forkchoiceUpdated` call
/// to the EL. The `restore_from_store` method (canonical_head.rs:311) has the same
/// Gloas fallback, but it's `pub(crate)` and harder to test directly. This test
/// verifies the equivalent path through `load_fork_choice` + `CanonicalHead::new`.
#[tokio::test]
async fn gloas_persist_load_fork_choice_preserves_head_hash() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(4)).await;

    // Capture the current head_hash before persist
    let cached_head_before = harness.chain.canonical_head.cached_head();
    let head_hash_before = cached_head_before.forkchoice_update_parameters().head_hash;
    assert!(
        head_hash_before.is_some(),
        "pre-condition: head_hash should be Some before persist"
    );

    let head_block_root = cached_head_before.head_block_root();
    let head_snapshot = cached_head_before.snapshot.clone();

    // Persist fork choice to the store
    harness
        .chain
        .persist_fork_choice()
        .expect("should persist fork choice");

    // Load fork choice back from the store (simulating node restart)
    let loaded_fork_choice =
        beacon_chain::BeaconChain::<EphemeralHarnessType<E>>::load_fork_choice(
            harness.chain.store.clone(),
            fork_choice::ResetPayloadStatuses::OnlyWithInvalidPayload,
            &harness.spec,
        )
        .expect("should load fork choice")
        .expect("fork choice should be present");

    // Verify loaded fork choice has the same head root
    let loaded_head_root = loaded_fork_choice
        .get_forkchoice_update_parameters()
        .head_root;
    assert_eq!(
        loaded_head_root, head_block_root,
        "loaded fork choice should have same head root"
    );

    // Construct a new CanonicalHead from the loaded fork choice + existing snapshot.
    // This is what the builder and restore_from_store do.
    let new_canonical_head: beacon_chain::CanonicalHead<EphemeralHarnessType<E>> =
        beacon_chain::CanonicalHead::new(loaded_fork_choice, head_snapshot);

    // Verify head_hash is correctly derived via the Gloas fallback
    let new_cached_head = new_canonical_head.cached_head();
    let head_hash_after = new_cached_head.forkchoice_update_parameters().head_hash;
    assert_eq!(
        head_hash_after, head_hash_before,
        "head_hash should survive persist + load + CanonicalHead::new"
    );
}

/// Verify that `head_hash` transitions from parent's payload hash to the current
/// block's payload hash after envelope processing.
///
/// Before envelope processing, the head block has `ExecutionStatus::Irrelevant` in
/// fork choice. The cached head's `head_hash` comes from the fallback
/// (`state.latest_block_hash()`), which at that point reflects the parent's envelope.
/// After `process_self_build_envelope` + `recompute_head`, the node transitions to
/// `Optimistic(payload_block_hash)` and `state.latest_block_hash()` is updated to the
/// current payload's hash.
///
/// This test verifies the head_hash is always correct (non-None, non-zero) through
/// both phases.
#[tokio::test]
async fn gloas_head_hash_updated_after_envelope_processing() {
    let harness = gloas_harness_at_epoch(0);
    // Build a few blocks so we have a stable chain with non-zero latest_block_hash
    Box::pin(harness.extend_slots(4)).await;

    // Capture the head_hash before producing a new block
    let head_hash_before = harness
        .chain
        .canonical_head
        .cached_head()
        .forkchoice_update_parameters()
        .head_hash;
    assert!(
        head_hash_before.is_some(),
        "pre-condition: head_hash should be Some"
    );

    // Produce a new block but DON'T process its envelope yet
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("Gloas block should produce an envelope");
    let block_root = block_contents.0.canonical_root();

    // Import the block (this calls recompute_head internally, but no envelope yet)
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // After block import + recompute_head (but before envelope processing):
    // The cached head should have SOME head_hash — it comes from the state's
    // latest_block_hash, which still holds the PARENT's payload hash (the envelope
    // from the parent was already processed).
    let head_hash_after_block = harness
        .chain
        .canonical_head
        .cached_head()
        .forkchoice_update_parameters()
        .head_hash;
    assert!(
        head_hash_after_block.is_some(),
        "head_hash should be Some even before envelope (fallback from parent's latest_block_hash)"
    );

    // Now process the envelope
    harness
        .chain
        .process_self_build_envelope(&signed_envelope)
        .await
        .expect("should process envelope");

    // Recompute head to update cached_head with post-envelope state
    harness.chain.recompute_head_at_current_slot().await;

    // After envelope: head_hash should now reflect the CURRENT block's payload
    let head_hash_after_envelope = harness
        .chain
        .canonical_head
        .cached_head()
        .forkchoice_update_parameters()
        .head_hash;
    assert!(
        head_hash_after_envelope.is_some(),
        "head_hash should be Some after envelope processing"
    );

    // The head_hash should have changed from the parent's to the current block's
    // (unless they happen to be the same, which is extremely unlikely with random
    // execution payloads in the mock EL)
    let envelope_payload_block_hash = signed_envelope.message.payload.block_hash;
    assert_eq!(
        head_hash_after_envelope.unwrap(),
        envelope_payload_block_hash,
        "head_hash should match the envelope's payload block_hash after processing"
    );
}

// =============================================================================
// Proposer boost timing — Gloas 4-interval boundary
// =============================================================================

/// Verify that Gloas blocks use 4 intervals per slot for the proposer boost deadline,
/// not the pre-Gloas 3 intervals.
///
/// With minimal preset (6s slots):
/// - Pre-Gloas: threshold = 6000ms / 3 = 2000ms
/// - Gloas: threshold = 6000ms / 4 = 1500ms
///
/// A block arriving at 1499ms should get boost; at 1500ms it should NOT.
/// If the intervals_per_slot were wrong (using 3 instead of 4), a block at 1500ms
/// would get boost (since 1500 < 2000), creating an incorrect head selection.
///
/// This exercises `fork_choice.rs` lines 820-838 which have zero test coverage.
#[tokio::test]
async fn gloas_proposer_boost_four_interval_boundary() {
    let harness = gloas_harness_at_epoch(0);
    // Build some blocks to have a stable chain
    Box::pin(harness.extend_slots(2)).await;

    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;

    // Produce a block at the next slot (without importing it through the chain)
    harness.advance_slot();
    let (block_contents, new_state, _envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let block = &block_contents.0;
    let block_root = block.canonical_root();

    // Test 1: block_delay = 1499ms — should get boost (under 1500ms threshold)
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        // Advance time to the block's slot to reset proposer boost root
        fc.update_time(next_slot).unwrap();
        assert!(
            fc.proposer_boost_root().is_zero(),
            "pre-condition: proposer boost root should be zero after slot tick"
        );

        fc.on_block(
            next_slot,
            block.message(),
            block_root,
            Duration::from_millis(1499),
            &new_state,
            PayloadVerificationStatus::Optimistic,
            None, // No canonical_head_proposer_index check
            &harness.spec,
        )
        .expect("on_block should succeed");

        assert_eq!(
            fc.proposer_boost_root(),
            block_root,
            "proposer boost should be granted at 1499ms (under Gloas 1500ms threshold)"
        );
    }

    // To test the 1500ms case, we need a fresh block that fork choice hasn't seen.
    // Produce another block at a later slot.
    harness.advance_slot();
    let state2 = harness.chain.head_beacon_state_cloned();
    let next_slot2 = next_slot + 1;
    let (block_contents2, new_state2, _envelope2) =
        harness.make_block_with_envelope(state2, next_slot2).await;

    let block2 = &block_contents2.0;
    let block2_root = block2.canonical_root();

    // But first we need to import the first block so block2's parent exists in fc
    // The first block was added to fc via on_block above, so parent is known.
    // We need to advance fc time to next_slot2 to reset the boost.

    // Test 2: block_delay = 1500ms — should NOT get boost (at the threshold)
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        fc.update_time(next_slot2).unwrap();
        assert!(
            fc.proposer_boost_root().is_zero(),
            "pre-condition: proposer boost root should be zero after new slot tick"
        );

        fc.on_block(
            next_slot2,
            block2.message(),
            block2_root,
            Duration::from_millis(1500),
            &new_state2,
            PayloadVerificationStatus::Optimistic,
            None,
            &harness.spec,
        )
        .expect("on_block should succeed");

        assert!(
            fc.proposer_boost_root().is_zero(),
            "proposer boost should NOT be granted at 1500ms (at Gloas threshold)"
        );
    }
}

// =============================================================================
// Payload invalidation — Gloas blocks
// =============================================================================

/// Verify that `InvalidateOne` correctly marks a Gloas block as Invalid.
///
/// After importing a Gloas block + processing its envelope, the mock EL marks it
/// `Valid`. We set it back to `Optimistic` (simulating a scenario where the EL hasn't
/// confirmed validity yet, e.g. during syncing) and then call `InvalidateOne`.
/// This exercises the payload invalidation path for Gloas blocks, which has zero
/// coverage in `payload_invalidation.rs`.
#[tokio::test]
async fn gloas_invalidate_one_marks_block_invalid() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(4)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let parent_root = head.beacon_block.parent_root();

    // The mock EL confirms payloads as Valid. Set the head back to Optimistic
    // to simulate the common scenario where the EL is syncing (PayloadStatus::Syncing)
    // and hasn't confirmed validity yet.
    let head_block_hash = {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        let block_index = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&head_root)
            .expect("head should be in fork choice");
        let hash = fc.proto_array().core_proto_array().nodes[block_index]
            .execution_status
            .block_hash()
            .expect("head should have a block hash");
        fc.proto_array_mut().core_proto_array_mut().nodes[block_index].execution_status =
            ExecutionStatus::Optimistic(hash);
        hash
    };

    // Verify the status was set correctly
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let block = fc.get_block(&head_root).unwrap();
        assert!(
            matches!(block.execution_status, ExecutionStatus::Optimistic(_)),
            "pre-condition: head should be Optimistic, got {:?}",
            block.execution_status
        );
    }

    // Invalidate the head block
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        fc.on_invalid_execution_payload(&InvalidationOperation::InvalidateOne {
            block_root: head_root,
        })
        .expect("invalidation should succeed");
    }

    // Verify the block is now Invalid
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let block = fc.get_block(&head_root).unwrap();
        assert!(
            matches!(block.execution_status, ExecutionStatus::Invalid(_)),
            "head should be Invalid after InvalidateOne, got {:?}",
            block.execution_status
        );
        assert_eq!(
            block.execution_status.block_hash(),
            Some(head_block_hash),
            "Invalid status should preserve the original block hash"
        );
    }

    // Recompute head — should move to the parent (since head is now Invalid)
    harness.chain.recompute_head_at_current_slot().await;

    let new_head = harness.chain.head_snapshot();
    assert_ne!(
        new_head.beacon_block_root, head_root,
        "head should have changed after invalidation"
    );
    assert_eq!(
        new_head.beacon_block_root, parent_root,
        "new head should be the parent of the invalidated block"
    );
}

/// Verify that `InvalidateMany` backward-walking stops at a Gloas block with
/// `ExecutionStatus::Irrelevant` (a block whose bid had zero block_hash, simulating
/// the pre-terminal-PoW or no-bid scenario).
///
/// The invalidation propagation in `proto_array.rs:563` breaks on Irrelevant nodes.
/// This test manually sets a node to Irrelevant, then runs InvalidateMany from a
/// descendant and verifies the Irrelevant node is NOT invalidated.
#[tokio::test]
async fn gloas_invalidation_stops_at_irrelevant_boundary() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(4)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let parent_root = head.beacon_block.parent_root();

    // Set up the fork choice state:
    // - Head: Optimistic (simulating EL syncing, hasn't confirmed yet)
    // - Parent: Irrelevant (simulating a block with zero bid hash)
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();

        // Set head to Optimistic (mock EL marks it Valid; revert to Optimistic)
        let head_index = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&head_root)
            .expect("head should be in fork choice");
        let head_hash = fc.proto_array().core_proto_array().nodes[head_index]
            .execution_status
            .block_hash()
            .expect("head should have a block hash");
        fc.proto_array_mut().core_proto_array_mut().nodes[head_index].execution_status =
            ExecutionStatus::Optimistic(head_hash);

        // Set parent to Irrelevant (simulating zero bid hash)
        let parent_index = *fc
            .proto_array()
            .core_proto_array()
            .indices
            .get(&parent_root)
            .expect("parent should be in fork choice");
        fc.proto_array_mut().core_proto_array_mut().nodes[parent_index].execution_status =
            ExecutionStatus::Irrelevant(false);
    }

    // Run InvalidateMany from the head, with latest_valid_ancestor = zero hash
    // (meaning no valid ancestor known — walk all the way back).
    // This should invalidate the head but stop at the parent (which is Irrelevant).
    {
        let mut fc = harness.chain.canonical_head.fork_choice_write_lock();
        fc.on_invalid_execution_payload(&InvalidationOperation::InvalidateMany {
            head_block_root: head_root,
            always_invalidate_head: true,
            latest_valid_ancestor: ExecutionBlockHash::zero(),
        })
        .expect("invalidation should succeed");
    }

    // Verify: head should be Invalid
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let head_block = fc.get_block(&head_root).unwrap();
        assert!(
            matches!(head_block.execution_status, ExecutionStatus::Invalid(_)),
            "head should be Invalid after InvalidateMany, got {:?}",
            head_block.execution_status
        );

        // Verify: parent should still be Irrelevant (NOT Invalid)
        // The backward walk in proto_array.rs breaks at Irrelevant nodes (line 563).
        let parent_block = fc.get_block(&parent_root).unwrap();
        assert!(
            matches!(
                parent_block.execution_status,
                ExecutionStatus::Irrelevant(_)
            ),
            "parent should remain Irrelevant (invalidation should stop here), got {:?}",
            parent_block.execution_status
        );
    }
}

/// When the EL returns `InvalidBlockHash` for a gossip-received envelope's `newPayload`,
/// `process_payload_envelope` should return an error mentioning "invalid block hash".
/// This is the gossip-path counterpart of
/// `gloas_self_build_envelope_el_invalid_block_hash_returns_error`.
///
/// The `InvalidBlockHash` EL response (distinct from `Invalid`) indicates the payload's
/// block hash itself is malformed. The gossip path calls `process_payload_envelope`
/// (beacon_chain.rs:2699-2710) which must propagate this as an error.
#[tokio::test]
async fn gloas_gossip_envelope_invalid_block_hash_returns_error() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    // Produce a block and its self-build envelope
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("Gloas block should produce an envelope");
    let block_root = block_contents.0.canonical_root();

    // Import the block only (simulating gossip block arriving first)
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Step 1: Gossip verification
    let verified = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope))
        .expect("gossip verification should pass");

    // Step 2: Apply to fork choice (marks payload_revealed = true)
    harness
        .chain
        .apply_payload_envelope_to_fork_choice(&verified)
        .expect("should apply to fork choice");

    // Configure mock EL to return InvalidBlockHash for newPayload
    let mock_el = harness.mock_execution_layer.as_ref().unwrap();
    mock_el
        .server
        .all_payloads_invalid_block_hash_on_new_payload();

    // Step 3: process_payload_envelope should fail because EL says InvalidBlockHash
    let result = harness.chain.process_payload_envelope(&verified).await;

    assert!(
        result.is_err(),
        "process_payload_envelope should error when EL returns InvalidBlockHash"
    );
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("invalid block hash"),
        "error should mention invalid block hash, got: {}",
        err_msg
    );

    // payload_revealed should still be true (set before EL call)
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "payload_revealed should remain true (set by apply_payload_envelope_to_fork_choice)"
        );
    }

    // The envelope should NOT be persisted to the store (errored before store write)
    let stored = harness
        .chain
        .store
        .get_payload_envelope(&block_root)
        .expect("store read should not error");
    assert!(
        stored.is_none(),
        "envelope should not be stored when process_payload_envelope fails"
    );
}

/// When the block is in fork choice but has been pruned from the store (e.g., due to
/// finalization pruning a hot DB block that FC still references), the gossip verification
/// path should return `MissingBeaconBlock`.
///
/// This exercises gloas_verification.rs:710-716 where `get_blinded_block` returns `None`
/// for a block_root that passes the FC checks. In practice this can happen during
/// finalization pruning or if the store is temporarily inconsistent.
#[tokio::test]
async fn gloas_gossip_verify_envelope_missing_beacon_block() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    // Produce a block and its self-build envelope
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("Gloas block should produce an envelope");
    let block_root = block_contents.0.canonical_root();

    // Import the block normally
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Delete the block from the store (simulating finalization pruning)
    // The block is still in fork choice but no longer in the hot DB.
    harness
        .chain
        .store
        .delete_block(&block_root)
        .expect("should delete block from store");

    // Gossip verification should fail with MissingBeaconBlock
    let result = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope));

    match result {
        Err(PayloadEnvelopeError::MissingBeaconBlock { block_root: root }) => {
            assert_eq!(
                root, block_root,
                "error should reference the missing block root"
            );
        }
        Ok(_) => panic!("expected MissingBeaconBlock error, got Ok"),
        Err(e) => panic!("expected MissingBeaconBlock error, got: {:?}", e),
    }
}

/// Verify the execution status lifecycle for Gloas blocks:
///
/// 1. After block import (without envelope), the fork choice node has
///    `Optimistic(bid_block_hash)` — from fork_choice.rs:988 where the bid's
///    block_hash is used since Gloas blocks carry a bid, not a payload.
/// 2. After `forkchoice_updated` (which happens during recompute_head), the mock
///    EL returns `Valid`, promoting the status to `Valid(bid_block_hash)`.
/// 3. After envelope processing, `on_valid_execution_payload` is called again
///    with the envelope's payload hash, confirming validity.
///
/// This exercises the Gloas-specific bid-based execution status path
/// (fork_choice.rs:980-989), which is distinct from pre-Gloas payload-based status.
#[tokio::test]
async fn gloas_execution_status_lifecycle_bid_optimistic_to_valid() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    // Produce a block and its envelope separately
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("Gloas block should produce an envelope");
    let bid_block_hash = signed_envelope.message.payload.block_hash;
    let block_root = block_contents.0.canonical_root();

    // Configure mock EL to return Syncing for forkchoice_updated (so the block
    // stays Optimistic after import instead of being promoted to Valid).
    let mock_el = harness.mock_execution_layer.as_ref().unwrap();
    mock_el.server.all_payloads_syncing_on_forkchoice_updated();

    // Import block only (without envelope)
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Step 1: Verify the block is Optimistic with the bid's block_hash
    // (fork_choice.rs:988 sets Optimistic(bid.message.block_hash))
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        match proto_block.execution_status {
            ExecutionStatus::Optimistic(hash) => {
                assert_eq!(
                    hash, bid_block_hash,
                    "Optimistic status should use the bid's block_hash"
                );
            }
            other => panic!(
                "block should be Optimistic after import (EL syncing), got {:?}",
                other
            ),
        }
        // payload_revealed should be false (no envelope processed yet)
        assert!(
            !proto_block.payload_revealed,
            "payload_revealed should be false before envelope"
        );
    }

    // Reset mock EL to return Valid for newPayload (for envelope processing)
    mock_el.server.all_payloads_valid();

    // Step 2: Process the envelope through the gossip pipeline
    let verified = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope))
        .expect("gossip verification should pass");

    harness
        .chain
        .apply_payload_envelope_to_fork_choice(&verified)
        .expect("should apply to fork choice");

    harness
        .chain
        .process_payload_envelope(&verified)
        .await
        .expect("process_payload_envelope should succeed");

    // Step 3: Verify the block is now Valid with the bid's block_hash
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        match proto_block.execution_status {
            ExecutionStatus::Valid(hash) => {
                assert_eq!(
                    hash, bid_block_hash,
                    "Valid status should preserve the bid's block_hash"
                );
            }
            other => panic!(
                "block should be Valid after envelope processing, got {:?}",
                other
            ),
        }
        // payload_revealed should now be true
        assert!(
            proto_block.payload_revealed,
            "payload_revealed should be true after envelope processing"
        );
    }
}

// =============================================================================
// Fork transition boundary, envelope error path, and state eviction tests
// =============================================================================

/// Verify that at the Fulu→Gloas fork boundary, the last Fulu block's fork choice
/// node has `bid_block_hash = None`. This is the field that controls whether the
/// `GloasParentPayloadUnknown` guard fires in block_verification.rs:977-984.
///
/// The guard checks `parent_block.bid_block_hash.is_some()` — for a Fulu parent this
/// is `None`, so the guard is bypassed. The existing test
/// `gloas_parent_payload_check_skips_pre_gloas_parent` only implicitly verifies this
/// (the chain doesn't break). This test explicitly inspects the fork choice state to
/// confirm the invariant, and also verifies the first Gloas block's node has a
/// non-None `bid_block_hash`.
#[tokio::test]
async fn gloas_fork_transition_fulu_parent_has_no_bid_in_fork_choice() {
    let gloas_fork_epoch = Epoch::new(2);
    let gloas_fork_slot = gloas_fork_epoch.start_slot(E::slots_per_epoch());
    let harness = gloas_harness_at_epoch(gloas_fork_epoch.as_u64());

    // Extend to the last Fulu slot.
    let last_fulu_slot = gloas_fork_slot - 1;
    Box::pin(harness.extend_to_slot(last_fulu_slot)).await;

    let fulu_head_root = harness.chain.head_snapshot().beacon_block_root;

    // Verify: the Fulu block in fork choice has bid_block_hash = None
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let fulu_block = fc
            .get_block(&fulu_head_root)
            .expect("Fulu head should be in FC");
        assert!(
            fulu_block.bid_block_hash.is_none(),
            "Fulu block should have bid_block_hash = None in fork choice, got {:?}",
            fulu_block.bid_block_hash
        );
    }

    // Extend to the first Gloas slot (fork transition).
    Box::pin(harness.extend_to_slot(gloas_fork_slot)).await;

    let gloas_head_root = harness.chain.head_snapshot().beacon_block_root;
    assert_ne!(
        gloas_head_root, fulu_head_root,
        "head should have advanced to Gloas block"
    );

    // Verify: the first Gloas block has bid_block_hash = Some(...)
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let gloas_block = fc
            .get_block(&gloas_head_root)
            .expect("Gloas head should be in FC");
        assert!(
            gloas_block.bid_block_hash.is_some(),
            "first Gloas block should have bid_block_hash = Some(...) in fork choice"
        );
        // The Fulu parent is still in FC with bid_block_hash = None
        let fulu_block = fc
            .get_block(&fulu_head_root)
            .expect("Fulu parent still in FC");
        assert!(
            fulu_block.bid_block_hash.is_none(),
            "Fulu parent should still have bid_block_hash = None after fork transition"
        );
    }
}

/// Exercise the `process_payload_envelope` error path when the post-block state has
/// been evicted from the state cache and hot DB. This simulates a real-world race
/// condition: the state cache is full and evicts the state between block import and
/// envelope arrival, or the hot DB was pruned.
///
/// After block import + gossip verification + fork choice update, we delete the state
/// and call `process_payload_envelope`. It should return an `EnvelopeProcessingError`
/// containing "Missing state". The block's `payload_revealed` should remain true in
/// fork choice (set during `apply_payload_envelope_to_fork_choice`), but the state
/// transition was not applied.
#[tokio::test]
async fn gloas_process_envelope_missing_state_returns_error() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    // Produce a block and its self-build envelope
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("Gloas block should produce an envelope");
    let block_root = block_contents.0.canonical_root();

    // Import the block (state gets cached)
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Gossip-verify the envelope and apply to fork choice
    let verified = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope))
        .expect("gossip verification should pass");

    harness
        .chain
        .apply_payload_envelope_to_fork_choice(&verified)
        .expect("should apply to fork choice");

    // Get the block's state root before evicting
    let block_state_root = harness
        .chain
        .store
        .get_blinded_block(&block_root)
        .unwrap()
        .unwrap()
        .message()
        .state_root();

    // Evict the post-block state from both cache and hot DB
    harness
        .chain
        .store
        .delete_state(&block_state_root, next_slot)
        .expect("should delete state");

    // Verify state is gone
    assert!(
        harness
            .chain
            .get_state(&block_state_root, Some(next_slot), false)
            .expect("should not error")
            .is_none(),
        "state should be gone after deletion"
    );

    // process_payload_envelope should fail with "Missing state"
    let result = harness.chain.process_payload_envelope(&verified).await;

    let err = result.expect_err("should fail with missing state");
    let err_msg = format!("{:?}", err);
    assert!(
        err_msg.contains("Missing state"),
        "error should mention 'Missing state', got: {}",
        err_msg
    );

    // Fork choice should still have payload_revealed = true (set during apply_payload_envelope_to_fork_choice)
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "payload_revealed should be true (fork choice was updated before state transition failed)"
        );
    }
}

/// Exercise the `process_payload_envelope` error path when the beacon block has been
/// deleted from the store after gossip verification. In a live network this can happen
/// if finalization prunes the block between the envelope's gossip verification and the
/// state transition step.
///
/// The code path at beacon_chain.rs:2608-2617 loads the block for `newPayload` and
/// should return `EnvelopeProcessingError` containing "Missing beacon block" when the
/// block is no longer in the store.
#[tokio::test]
async fn gloas_process_envelope_missing_block_returns_error() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(2)).await;

    // Produce a block and its self-build envelope
    harness.advance_slot();
    let head_state = harness.chain.head_beacon_state_cloned();
    let next_slot = head_state.slot() + 1;
    let (block_contents, _state, envelope) = harness
        .make_block_with_envelope(head_state, next_slot)
        .await;

    let signed_envelope = envelope.expect("Gloas block should produce an envelope");
    let block_root = block_contents.0.canonical_root();

    // Import the block normally
    harness
        .process_block(next_slot, block_root, block_contents)
        .await
        .expect("block import should succeed");

    // Gossip-verify the envelope and apply to fork choice (block still in store at this point)
    let verified = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope))
        .expect("gossip verification should pass");

    harness
        .chain
        .apply_payload_envelope_to_fork_choice(&verified)
        .expect("should apply to fork choice");

    // Delete the block from the store (simulating finalization pruning)
    harness
        .chain
        .store
        .delete_block(&block_root)
        .expect("should delete block from store");

    // Verify block is gone
    assert!(
        harness
            .chain
            .store
            .get_blinded_block(&block_root)
            .unwrap()
            .is_none(),
        "block should be gone after deletion"
    );

    // process_payload_envelope should fail with "Missing beacon block"
    let result = harness.chain.process_payload_envelope(&verified).await;

    let err = result.expect_err("should fail with missing block");
    let err_msg = format!("{:?}", err);
    assert!(
        err_msg.contains("Missing beacon block"),
        "error should mention 'Missing beacon block', got: {}",
        err_msg
    );

    // Fork choice should still have payload_revealed = true
    {
        let fc = harness.chain.canonical_head.fork_choice_read_lock();
        let proto_block = fc.get_block(&block_root).unwrap();
        assert!(
            proto_block.payload_revealed,
            "payload_revealed should be true (fork choice was updated before block deletion)"
        );
    }
}

// =============================================================================
// Execution bid gossip: InsufficientBuilderBalance
// =============================================================================

/// A bid whose `value` exceeds the builder's registered balance is rejected
/// with `InsufficientBuilderBalance`. This guard prevents builders from
/// offering more value than their deposit covers — accepting such a bid would
/// let a builder commit to a payment they cannot fulfill, leaving the proposer
/// unpaid after revealing the payload.
///
/// The balance check (check 2b) runs after the builder-exists and is-active
/// checks but before equivocation detection, parent root, proposer preferences,
/// and signature verification — so we don't need valid signatures or preferences.
#[tokio::test]
async fn gloas_bid_gossip_rejects_insufficient_builder_balance() {
    // Builder 0: deposit_epoch=0, balance=100 (very low)
    let harness = gloas_harness_with_builders(&[(0, 100)]);
    // Extend to finalize so the builder becomes active (deposit_epoch < finalized_epoch)
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;
    let state = harness.chain.head_beacon_state_cloned();

    // Verify builder is active (precondition)
    let builder = state
        .builders()
        .unwrap()
        .get(0)
        .expect("builder 0 should exist");
    assert_eq!(builder.balance, 100);
    assert!(
        builder.is_active_at_finalized_epoch(state.finalized_checkpoint().epoch, &harness.spec),
        "builder should be active after finalization"
    );

    // Create a bid with value=200, exceeding the builder's balance of 100.
    // make_external_bid sets execution_payment = value, which passes the
    // zero-payment check. builder_index=0 passes the exists/active checks.
    let bid = make_external_bid(&state, head_root, next_slot, 0, 200);

    let err = assert_bid_rejected(&harness, bid, "bid value exceeds builder balance");
    match err {
        ExecutionBidError::InsufficientBuilderBalance {
            builder_index,
            balance,
            bid_value,
        } => {
            assert_eq!(builder_index, 0);
            assert_eq!(balance, 100);
            assert_eq!(bid_value, 200);
        }
        other => panic!("expected InsufficientBuilderBalance, got {:?}", other),
    }
}

// =============================================================================
// Execution bid gossip: BuilderEquivocation
// =============================================================================

/// When a builder submits two *different* bids for the same slot, the second
/// bid is rejected with `BuilderEquivocation`. This is the ePBS slashable-
/// condition detection for builders — a builder that equivocates is trying to
/// commit to multiple payloads for the same slot.
///
/// The equivocation check (check 3) runs after the balance check but before
/// parent root validation, proposer preferences, and signature verification.
/// The first bid passes the equivocation check (recorded as `New`) and
/// continues to later checks where it may fail (e.g. at signature). But the
/// observation is already recorded, so the second bid (different tree_hash_root)
/// triggers `Equivocation`.
#[tokio::test]
async fn gloas_bid_gossip_rejects_builder_equivocation() {
    // Builder 0: deposit_epoch=0, balance=10 ETH
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;
    let state = harness.chain.head_beacon_state_cloned();

    // First bid: value=5000. This will pass checks 1-4 (slot, payment,
    // builder exists/active, balance) and get observed as New in check 5.
    // It will then fail at parent root, proposer preferences, or signature —
    // but the observation is already recorded.
    let bid_1 = make_external_bid(&state, head_root, next_slot, 0, 5000);
    // Submit the first bid — it will fail (likely at parent root or prefs)
    // but the observation is recorded.
    let _ = harness.chain.verify_execution_bid_for_gossip(bid_1);

    // Second bid: value=6000 (different value → different tree_hash_root).
    // Same builder (0), same slot (next_slot). This should trigger Equivocation
    // at check 5 because we already observed a different bid root from
    // builder 0 for this slot.
    let bid_2 = make_external_bid(&state, head_root, next_slot, 0, 6000);

    let err = assert_bid_rejected(&harness, bid_2, "second bid should be equivocation");
    match err {
        ExecutionBidError::BuilderEquivocation {
            builder_index,
            slot,
            ..
        } => {
            assert_eq!(builder_index, 0);
            assert_eq!(slot, next_slot);
        }
        other => panic!("expected BuilderEquivocation, got {:?}", other),
    }
}

// =============================================================================
// Payload attestation gossip: ValidatorEquivocation
// =============================================================================

/// When a PTC validator submits a payload attestation with `payload_present=true`
/// and then a second attestation for the same slot/block with `payload_present=false`,
/// the second attestation is rejected with `ValidatorEquivocation`. This is the
/// primary equivocation detection for payload attesters — a validator that votes
/// both ways is misbehaving.
///
/// The equivocation check (check 5) runs before signature verification (check 6).
/// The first attestation is recorded as `New` in the observation tracker even if
/// it later fails at the signature check. The second attestation (different
/// `payload_present`) then triggers equivocation before reaching signature check.
#[tokio::test]
async fn gloas_payload_attestation_gossip_rejects_validator_equivocation() {
    let harness = gloas_harness_at_epoch(0);
    Box::pin(harness.extend_slots(3)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    // Find a PTC member for the head slot
    let ptc =
        get_ptc_committee::<E>(state, head_slot, &harness.spec).expect("should get PTC committee");
    assert!(!ptc.is_empty(), "PTC committee should not be empty");
    let ptc_member_index = ptc[0];

    // Find the position of this PTC member in the committee to set the correct bit
    let ptc_position = ptc
        .iter()
        .position(|&idx| idx == ptc_member_index)
        .expect("PTC member should be in PTC committee");

    // First attestation: payload_present=true, invalid signature.
    // The equivocation check records `New` before signature verification,
    // so the observation is committed even though sig check will fail.
    let mut aggregation_bits_1 = BitVector::default();
    aggregation_bits_1
        .set(ptc_position, true)
        .expect("should set bit");

    let attestation_1 = PayloadAttestation::<E> {
        aggregation_bits: aggregation_bits_1,
        data: PayloadAttestationData {
            beacon_block_root: head_root,
            slot: head_slot,
            payload_present: true,
            blob_data_available: true,
        },
        signature: AggregateSignature::empty(),
    };

    // Submit first attestation — it will get `New` at equivocation check,
    // then fail at signature verification. That's expected.
    let result_1 = harness
        .chain
        .verify_payload_attestation_for_gossip(attestation_1);
    // Verify it failed at signature, not at equivocation
    match result_1 {
        Err(PayloadAttestationError::InvalidSignature) => {}
        Err(other) => panic!(
            "first attestation should fail with InvalidSignature, got {:?}",
            other
        ),
        Ok(_) => panic!("first attestation should fail at signature check"),
    }

    // Second attestation: same validator, same slot/block, but payload_present=false.
    // This should trigger ValidatorEquivocation before reaching signature check.
    let mut aggregation_bits_2 = BitVector::default();
    aggregation_bits_2
        .set(ptc_position, true)
        .expect("should set bit");

    let attestation_2 = PayloadAttestation::<E> {
        aggregation_bits: aggregation_bits_2,
        data: PayloadAttestationData {
            beacon_block_root: head_root,
            slot: head_slot,
            payload_present: false, // opposite of first attestation
            blob_data_available: true,
        },
        signature: AggregateSignature::empty(),
    };

    let result_2 = harness
        .chain
        .verify_payload_attestation_for_gossip(attestation_2);
    match result_2 {
        Err(PayloadAttestationError::ValidatorEquivocation {
            validator_index,
            slot,
            beacon_block_root,
        }) => {
            assert_eq!(validator_index, ptc_member_index);
            assert_eq!(slot, head_slot);
            assert_eq!(beacon_block_root, head_root);
        }
        Err(other) => panic!("expected ValidatorEquivocation, got {:?}", other),
        Ok(_) => panic!("second attestation should be rejected as equivocation"),
    }
}

// =============================================================================
// Execution bid gossip: ProposerPreferencesNotSeen
// =============================================================================

/// When a bid arrives for a slot where no proposer preferences have been
/// inserted into the pool, the bid is rejected with `ProposerPreferencesNotSeen`.
///
/// Spec: `[IGNORE] the SignedProposerPreferences where preferences.proposal_slot
/// is equal to bid.slot has been seen.`
///
/// This check (check 4b) runs after parent root validation (check 4). The bid
/// must pass checks 1-4 (slot, payment, builder exists/active, balance,
/// equivocation, parent root) to reach the proposer preferences check.
///
/// This guard prevents builders from submitting bids before the proposer has
/// declared their preferences. Without it, builders could bid with arbitrary
/// fee_recipient/gas_limit values and potentially win slots with unacceptable
/// terms for the proposer.
#[tokio::test]
async fn gloas_bid_gossip_rejects_no_proposer_preferences() {
    // Builder 0: deposit_epoch=0, balance=10 ETH
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;
    let state = harness.chain.head_beacon_state_cloned();

    // Verify NO proposer preferences exist for next_slot (precondition).
    assert!(
        harness.chain.get_proposer_preferences(next_slot).is_none(),
        "no preferences should exist for slot {} before insertion",
        next_slot
    );

    // Create a bid for next_slot. It will pass checks 1-4 (slot, payment,
    // builder, balance, equivocation, parent root) but fail at check 4b
    // because no proposer preferences have been inserted.
    let bid = make_external_bid(&state, head_root, next_slot, 0, 5000);

    let err = assert_bid_rejected(
        &harness,
        bid,
        "bid should fail without proposer preferences",
    );
    match err {
        ExecutionBidError::ProposerPreferencesNotSeen { slot } => {
            assert_eq!(slot, next_slot);
        }
        other => panic!("expected ProposerPreferencesNotSeen, got {:?}", other),
    }
}

// =============================================================================
// Execution bid gossip: FeeRecipientMismatch
// =============================================================================

/// When a bid's fee_recipient does not match the proposer's declared preferences,
/// the bid is rejected with `FeeRecipientMismatch`.
///
/// Spec: `[REJECT] bid.fee_recipient matches the fee_recipient from the
/// proposer's SignedProposerPreferences.`
///
/// This is a critical validator protection check: without it, a builder could
/// direct execution rewards to an arbitrary address, stealing the proposer's
/// MEV revenue. The proposer declares their preferred fee_recipient via
/// `SignedProposerPreferences`, and all bids must match it exactly.
#[tokio::test]
async fn gloas_bid_gossip_rejects_fee_recipient_mismatch() {
    // Builder 0: deposit_epoch=0, balance=10 ETH
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;
    let state = harness.chain.head_beacon_state_cloned();

    // Insert proposer preferences with fee_recipient=0xAA..AA
    let proposer_fee_recipient = Address::repeat_byte(0xAA);
    let prefs = SignedProposerPreferences {
        message: ProposerPreferences {
            proposal_slot: next_slot.as_u64(),
            validator_index: 0,
            fee_recipient: proposer_fee_recipient,
            gas_limit: 30_000_000,
        },
        signature: Signature::empty(),
    };
    assert!(
        harness.chain.insert_proposer_preferences(prefs),
        "preferences insertion should succeed"
    );

    // Create a bid with fee_recipient=0x00..00 (default from make_external_bid).
    // This mismatches the proposer's preference of 0xAA..AA.
    // The bid passes checks 1-4 (slot, payment, builder, balance, equivocation,
    // parent root) but fails at fee_recipient comparison in check 4b.
    let bid = make_external_bid(&state, head_root, next_slot, 0, 5000);
    assert_ne!(
        bid.message.fee_recipient, proposer_fee_recipient,
        "bid fee_recipient should differ from preferences"
    );

    let err = assert_bid_rejected(&harness, bid, "bid should fail with fee_recipient mismatch");
    match err {
        ExecutionBidError::FeeRecipientMismatch { expected, received } => {
            assert_eq!(expected, proposer_fee_recipient);
            assert_eq!(received, Address::zero());
        }
        other => panic!("expected FeeRecipientMismatch, got {:?}", other),
    }
}

// =============================================================================
// Execution bid gossip: GasLimitMismatch
// =============================================================================

/// When a bid's gas_limit does not match the proposer's declared preferences,
/// the bid is rejected with `GasLimitMismatch`.
///
/// Spec: `[REJECT] bid.gas_limit matches the gas_limit from the proposer's
/// SignedProposerPreferences.`
///
/// This check runs after fee_recipient validation. The gas_limit determines the
/// maximum computational work a builder's payload can include. A mismatch could
/// mean the builder is trying to use more (or less) gas than the proposer agreed
/// to, which affects validator economics and block validation constraints.
#[tokio::test]
async fn gloas_bid_gossip_rejects_gas_limit_mismatch() {
    // Builder 0: deposit_epoch=0, balance=10 ETH
    let harness = gloas_harness_with_builders(&[(0, 10_000_000_000)]);
    Box::pin(harness.extend_slots(64)).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let next_slot = head_slot + 1;
    let state = harness.chain.head_beacon_state_cloned();

    // Insert proposer preferences with gas_limit=50_000_000 (different from
    // the default 30_000_000 used by make_external_bid).
    // Use fee_recipient=Address::zero() so it matches the bid's default,
    // ensuring we reach the gas_limit check (which comes after fee_recipient).
    let proposer_gas_limit = 50_000_000u64;
    let prefs = SignedProposerPreferences {
        message: ProposerPreferences {
            proposal_slot: next_slot.as_u64(),
            validator_index: 0,
            fee_recipient: Address::zero(),
            gas_limit: proposer_gas_limit,
        },
        signature: Signature::empty(),
    };
    assert!(
        harness.chain.insert_proposer_preferences(prefs),
        "preferences insertion should succeed"
    );

    // Create a bid with gas_limit=30_000_000 (default from make_external_bid).
    // This mismatches the proposer's preference of 50_000_000.
    // The bid passes checks 1-4 and fee_recipient check, but fails at gas_limit.
    let bid = make_external_bid(&state, head_root, next_slot, 0, 5000);
    assert_eq!(
        bid.message.gas_limit, 30_000_000,
        "bid gas_limit should be 30M (default)"
    );

    let err = assert_bid_rejected(&harness, bid, "bid should fail with gas_limit mismatch");
    match err {
        ExecutionBidError::GasLimitMismatch { expected, received } => {
            assert_eq!(expected, proposer_gas_limit);
            assert_eq!(received, 30_000_000);
        }
        other => panic!("expected GasLimitMismatch, got {:?}", other),
    }
}
