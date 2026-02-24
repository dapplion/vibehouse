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
