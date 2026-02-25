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

use beacon_chain::BeaconChainError;
use beacon_chain::BlockError;
use beacon_chain::test_utils::{BeaconChainHarness, EphemeralHarnessType};
use state_processing::per_block_processing::gloas::get_ptc_committee;
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
