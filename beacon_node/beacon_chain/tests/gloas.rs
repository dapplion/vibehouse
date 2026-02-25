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
use beacon_chain::execution_proof_verification::GossipExecutionProofError;
use beacon_chain::gloas_verification::{ExecutionBidError, PayloadEnvelopeError};
use beacon_chain::test_utils::{BeaconChainHarness, EphemeralHarnessType};
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
