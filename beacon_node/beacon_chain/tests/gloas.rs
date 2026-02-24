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

use beacon_chain::BlockError;
use beacon_chain::test_utils::{BeaconChainHarness, EphemeralHarnessType};
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
