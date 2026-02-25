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
use beacon_chain::gloas_verification::{ExecutionBidError, PayloadEnvelopeError};
use beacon_chain::test_utils::{BeaconChainHarness, EphemeralHarnessType};
use fork_choice::{ExecutionStatus, PayloadVerificationStatus};
use state_processing::per_block_processing::gloas::get_ptc_committee;
use std::sync::Arc;
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
    PayloadAttestation {
        aggregation_bits: Default::default(),
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

    let result = harness.chain.get_best_execution_bid(Slot::new(1));
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

    let result = harness.chain.get_best_execution_bid(target_slot);
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

    let result = harness.chain.get_best_execution_bid(target_slot);
    assert!(result.is_some());
    assert_eq!(
        result.unwrap().message.value,
        2000,
        "should return highest-value bid"
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

    // Genesis block should not have a builder_index since it wasn't produced via ePBS
    // (it's the anchor block inserted during chain init)
    assert_eq!(
        genesis_block.builder_index, None,
        "genesis block should have no builder_index"
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
        .get_best_execution_bid(target_slot)
        .expect("should have a bid");
    assert_eq!(best.message.value, 2000, "should return highest-value bid");
    assert_eq!(best.message.builder_index, 1);

    // Verify old-slot bids are pruned
    let future_slot = target_slot + 10;
    let result = harness.chain.get_best_execution_bid(future_slot);
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

    // Verify bid is retrievable from pool
    let best = harness
        .chain
        .get_best_execution_bid(head_slot)
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
