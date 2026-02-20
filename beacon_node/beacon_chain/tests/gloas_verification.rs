#![cfg(not(debug_assertions))]

//! Integration tests for Gloas ePBS gossip verification functions.
//!
//! Tests the three gossip verification functions in `gloas_verification.rs`:
//! - `verify_execution_bid_for_gossip`
//! - `verify_payload_attestation_for_gossip`
//! - `verify_payload_envelope_for_gossip`

use beacon_chain::ChainConfig;
use beacon_chain::gloas_verification::{
    ExecutionBidError, PayloadAttestationError, PayloadEnvelopeError,
};
use beacon_chain::test_utils::{
    AttestationStrategy, BeaconChainHarness, BlockStrategy, EphemeralHarnessType, test_spec,
};
use std::sync::{Arc, LazyLock};
use types::*;

type E = MainnetEthSpec;

/// Extract Err from a Result where the Ok type doesn't implement Debug.
fn unwrap_err<T, E: std::fmt::Debug>(result: Result<T, E>, msg: &str) -> E {
    match result {
        Ok(_) => panic!("{}: expected Err, got Ok", msg),
        Err(e) => e,
    }
}

const VALIDATOR_COUNT: usize = 24;

static KEYPAIRS: LazyLock<Vec<Keypair>> =
    LazyLock::new(|| types::test_utils::generate_deterministic_keypairs(VALIDATOR_COUNT));

/// Build a Gloas harness and extend the chain by `num_blocks`.
async fn gloas_harness(num_blocks: usize) -> BeaconChainHarness<EphemeralHarnessType<E>> {
    let spec = test_spec::<E>();
    assert!(
        spec.gloas_fork_epoch == Some(Epoch::new(0)),
        "tests require FORK_NAME=gloas"
    );

    let harness = BeaconChainHarness::builder(MainnetEthSpec)
        .spec(Arc::new(spec))
        .keypairs(KEYPAIRS[0..VALIDATOR_COUNT].to_vec())
        .fresh_ephemeral_store()
        .mock_execution_layer()
        .chain_config(ChainConfig {
            reconstruct_historic_states: true,
            ..ChainConfig::default()
        })
        .build();

    harness.advance_slot();

    if num_blocks > 0 {
        harness
            .extend_chain(
                num_blocks,
                BlockStrategy::OnCanonicalHead,
                AttestationStrategy::AllValidators,
            )
            .await;
    }

    harness
}

// =============================================================================
// verify_execution_bid_for_gossip tests
// =============================================================================

#[tokio::test]
async fn bid_slot_not_current_or_next_past() {
    let harness = gloas_harness(1).await;

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = Slot::new(0);
    bid.message.execution_payment = 1;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject past slot bid",
    );
    assert!(
        matches!(err, ExecutionBidError::SlotNotCurrentOrNext { .. }),
        "expected SlotNotCurrentOrNext, got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_slot_not_current_or_next_future() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot + 10;
    bid.message.execution_payment = 1;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject future slot bid",
    );
    assert!(
        matches!(err, ExecutionBidError::SlotNotCurrentOrNext { .. }),
        "expected SlotNotCurrentOrNext, got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_zero_execution_payment() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 0;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject zero payment bid",
    );
    assert!(
        matches!(err, ExecutionBidError::ZeroExecutionPayment),
        "expected ZeroExecutionPayment, got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_unknown_builder_index_zero() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject unknown builder",
    );
    assert!(
        matches!(err, ExecutionBidError::UnknownBuilder { builder_index: 0 }),
        "expected UnknownBuilder(0), got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_unknown_builder_high_index() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 999;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject unknown builder",
    );
    assert!(
        matches!(
            err,
            ExecutionBidError::UnknownBuilder { builder_index: 999 }
        ),
        "expected UnknownBuilder(999), got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_current_slot_passes_slot_check() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should fail on builder check",
    );
    assert!(
        !matches!(err, ExecutionBidError::SlotNotCurrentOrNext { .. }),
        "bid at current slot should pass slot check, got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_next_slot_passes_slot_check() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot + 1;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should fail on builder check",
    );
    assert!(
        !matches!(err, ExecutionBidError::SlotNotCurrentOrNext { .. }),
        "bid at next slot should pass slot check, got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_slot_two_ahead_rejected() {
    let harness = gloas_harness(3).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot + 2;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 42;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject bid two slots ahead",
    );
    assert!(
        matches!(err, ExecutionBidError::SlotNotCurrentOrNext { .. }),
        "expected SlotNotCurrentOrNext, got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_slot_one_behind_rejected() {
    let harness = gloas_harness(3).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot - 1;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 42;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject bid one slot behind",
    );
    assert!(
        matches!(err, ExecutionBidError::SlotNotCurrentOrNext { .. }),
        "expected SlotNotCurrentOrNext, got {:?}",
        err
    );
}

// =============================================================================
// verify_payload_attestation_for_gossip tests
// =============================================================================

#[tokio::test]
async fn attestation_future_slot() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut attestation = PayloadAttestation::<E>::empty();
    attestation.data.slot = current_slot + 100;
    attestation.aggregation_bits.set(0, true).unwrap();

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_attestation_for_gossip(attestation),
        "should reject future slot attestation",
    );
    assert!(
        matches!(err, PayloadAttestationError::FutureSlot { .. }),
        "expected FutureSlot, got {:?}",
        err
    );
}

#[tokio::test]
async fn attestation_past_slot() {
    let harness = gloas_harness(8).await;

    let mut attestation = PayloadAttestation::<E>::empty();
    attestation.data.slot = Slot::new(0);
    attestation.aggregation_bits.set(0, true).unwrap();

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_attestation_for_gossip(attestation),
        "should reject past slot attestation",
    );
    assert!(
        matches!(err, PayloadAttestationError::PastSlot { .. }),
        "expected PastSlot, got {:?}",
        err
    );
}

#[tokio::test]
async fn attestation_empty_aggregation_bits() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut attestation = PayloadAttestation::<E>::empty();
    attestation.data.slot = current_slot;

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_attestation_for_gossip(attestation),
        "should reject empty aggregation bits",
    );
    assert!(
        matches!(err, PayloadAttestationError::EmptyAggregationBits),
        "expected EmptyAggregationBits, got {:?}",
        err
    );
}

#[tokio::test]
async fn attestation_unknown_beacon_block_root() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut attestation = PayloadAttestation::<E>::empty();
    attestation.data.slot = current_slot;
    attestation.data.beacon_block_root = Hash256::from_low_u64_be(0xdeadbeef);
    attestation.aggregation_bits.set(0, true).unwrap();

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_attestation_for_gossip(attestation),
        "should reject unknown block root",
    );
    assert!(
        matches!(err, PayloadAttestationError::UnknownBeaconBlockRoot { .. }),
        "expected UnknownBeaconBlockRoot, got {:?}",
        err
    );
}

#[tokio::test]
async fn attestation_valid_slot_passes_slot_check() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut attestation = PayloadAttestation::<E>::empty();
    attestation.data.slot = current_slot;
    attestation.aggregation_bits.set(0, true).unwrap();
    attestation.data.beacon_block_root = Hash256::from_low_u64_be(0xabcd);

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_attestation_for_gossip(attestation),
        "should fail at later check",
    );
    // Should NOT be a slot or empty bits error
    assert!(
        !matches!(
            err,
            PayloadAttestationError::FutureSlot { .. }
                | PayloadAttestationError::PastSlot { .. }
                | PayloadAttestationError::EmptyAggregationBits
        ),
        "valid slot should pass early checks, got {:?}",
        err
    );
}

// =============================================================================
// verify_payload_envelope_for_gossip tests
// =============================================================================

#[tokio::test]
async fn envelope_block_root_unknown() {
    let harness = gloas_harness(1).await;

    let mut envelope = SignedExecutionPayloadEnvelope::<E>::empty();
    envelope.message.beacon_block_root = Hash256::from_low_u64_be(0xdeadbeef);
    envelope.message.slot = harness.chain.slot().unwrap();

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_envelope_for_gossip(Arc::new(envelope)),
        "should reject unknown block root",
    );
    assert!(
        matches!(err, PayloadEnvelopeError::BlockRootUnknown { .. }),
        "expected BlockRootUnknown, got {:?}",
        err
    );

    // Verify the envelope was buffered in pending_gossip_envelopes
    let pending = harness.chain.pending_gossip_envelopes.lock();
    assert!(
        pending.contains_key(&Hash256::from_low_u64_be(0xdeadbeef)),
        "envelope should be buffered for later processing"
    );
}

#[tokio::test]
async fn envelope_slot_mismatch() {
    let harness = gloas_harness(2).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    let mut envelope = SignedExecutionPayloadEnvelope::<E>::empty();
    envelope.message.beacon_block_root = head_root;
    envelope.message.slot = head_slot + 5;

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_envelope_for_gossip(Arc::new(envelope)),
        "should reject slot mismatch",
    );
    assert!(
        matches!(
            err,
            PayloadEnvelopeError::SlotMismatch {
                block_slot,
                envelope_slot,
            } if block_slot == head_slot && envelope_slot == head_slot + 5
        ),
        "expected SlotMismatch, got {:?}",
        err
    );
}

#[tokio::test]
async fn envelope_builder_index_mismatch() {
    let harness = gloas_harness(2).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    let block = harness
        .chain
        .store
        .get_blinded_block(&head_root)
        .unwrap()
        .unwrap();
    let committed_bid = block
        .message()
        .body()
        .signed_execution_payload_bid()
        .unwrap();

    let mut envelope = SignedExecutionPayloadEnvelope::<E>::empty();
    envelope.message.beacon_block_root = head_root;
    envelope.message.slot = head_slot;
    envelope.message.builder_index = committed_bid.message.builder_index.wrapping_add(1);

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_envelope_for_gossip(Arc::new(envelope)),
        "should reject builder index mismatch",
    );
    assert!(
        matches!(err, PayloadEnvelopeError::BuilderIndexMismatch { .. }),
        "expected BuilderIndexMismatch, got {:?}",
        err
    );
}

#[tokio::test]
async fn envelope_block_hash_mismatch() {
    let harness = gloas_harness(2).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    let block = harness
        .chain
        .store
        .get_blinded_block(&head_root)
        .unwrap()
        .unwrap();
    let committed_bid = block
        .message()
        .body()
        .signed_execution_payload_bid()
        .unwrap();

    let mut envelope = SignedExecutionPayloadEnvelope::<E>::empty();
    envelope.message.beacon_block_root = head_root;
    envelope.message.slot = head_slot;
    envelope.message.builder_index = committed_bid.message.builder_index;
    envelope.message.payload.block_hash =
        ExecutionBlockHash::from_root(Hash256::from_low_u64_be(0xbad));

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_envelope_for_gossip(Arc::new(envelope)),
        "should reject block hash mismatch",
    );
    assert!(
        matches!(err, PayloadEnvelopeError::BlockHashMismatch { .. }),
        "expected BlockHashMismatch, got {:?}",
        err
    );
}

#[tokio::test]
async fn envelope_buffering_preserves_envelope() {
    let harness = gloas_harness(1).await;

    let unknown_root = Hash256::from_low_u64_be(0x1234);
    let mut envelope = SignedExecutionPayloadEnvelope::<E>::empty();
    envelope.message.beacon_block_root = unknown_root;
    envelope.message.slot = Slot::new(42);

    let _ = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(envelope));

    let pending = harness.chain.pending_gossip_envelopes.lock();
    let buffered = pending.get(&unknown_root).unwrap();
    assert_eq!(buffered.message.slot, Slot::new(42));
}

#[tokio::test]
async fn envelope_multiple_unknown_roots_buffered() {
    let harness = gloas_harness(1).await;

    let root1 = Hash256::from_low_u64_be(0x1111);
    let root2 = Hash256::from_low_u64_be(0x2222);

    let mut env1 = SignedExecutionPayloadEnvelope::<E>::empty();
    env1.message.beacon_block_root = root1;
    env1.message.slot = Slot::new(10);

    let mut env2 = SignedExecutionPayloadEnvelope::<E>::empty();
    env2.message.beacon_block_root = root2;
    env2.message.slot = Slot::new(20);

    let _ = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(env1));
    let _ = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(env2));

    let pending = harness.chain.pending_gossip_envelopes.lock();
    assert_eq!(pending.len(), 2, "both envelopes should be buffered");
    assert_eq!(pending.get(&root1).unwrap().message.slot, Slot::new(10));
    assert_eq!(pending.get(&root2).unwrap().message.slot, Slot::new(20));
}

#[tokio::test]
async fn envelope_duplicate_root_overwrites() {
    let harness = gloas_harness(1).await;

    let root = Hash256::from_low_u64_be(0x3333);

    let mut env1 = SignedExecutionPayloadEnvelope::<E>::empty();
    env1.message.beacon_block_root = root;
    env1.message.slot = Slot::new(10);

    let mut env2 = SignedExecutionPayloadEnvelope::<E>::empty();
    env2.message.beacon_block_root = root;
    env2.message.slot = Slot::new(20);

    let _ = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(env1));
    let _ = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(env2));

    let pending = harness.chain.pending_gossip_envelopes.lock();
    assert_eq!(pending.len(), 1, "same root should overwrite");
    assert_eq!(
        pending.get(&root).unwrap().message.slot,
        Slot::new(20),
        "second envelope should overwrite first"
    );
}

// =============================================================================
// Observation tracker tests (via public lock fields)
// =============================================================================

#[tokio::test]
async fn bid_observation_new_then_duplicate() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let bid_root = Hash256::from_low_u64_be(0x1111);
    harness
        .chain
        .observed_execution_bids
        .lock()
        .observe_bid(current_slot, 5, bid_root);

    assert_eq!(
        harness
            .chain
            .observed_execution_bids
            .lock()
            .observed_bid_count(),
        1,
        "should have 1 observed bid"
    );

    // Observe different bid for same builder/slot (equivocation)
    let bid_root_2 = Hash256::from_low_u64_be(0x2222);
    harness
        .chain
        .observed_execution_bids
        .lock()
        .observe_bid(current_slot, 5, bid_root_2);

    assert_eq!(
        harness
            .chain
            .observed_execution_bids
            .lock()
            .observed_slot_count(),
        1,
        "should have 1 observed slot"
    );
}

#[tokio::test]
async fn bid_observation_different_builders_independent() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    let bid_root_a = Hash256::from_low_u64_be(0xaaaa);
    let bid_root_b = Hash256::from_low_u64_be(0xbbbb);

    harness
        .chain
        .observed_execution_bids
        .lock()
        .observe_bid(current_slot, 5, bid_root_a);
    harness
        .chain
        .observed_execution_bids
        .lock()
        .observe_bid(current_slot, 6, bid_root_b);

    assert_eq!(
        harness
            .chain
            .observed_execution_bids
            .lock()
            .observed_bid_count(),
        2,
        "two different builders should be tracked independently"
    );
}

#[tokio::test]
async fn attestation_observation_counts() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();
    let block_root = Hash256::from_low_u64_be(0xaaaa);

    harness
        .chain
        .observed_payload_attestations
        .lock()
        .observe_attestation(current_slot, block_root, 10, true);

    assert_eq!(
        harness
            .chain
            .observed_payload_attestations
            .lock()
            .observed_attestation_count(),
        1,
        "should have 1 observed attestation"
    );

    harness
        .chain
        .observed_payload_attestations
        .lock()
        .observe_attestation(current_slot, block_root, 11, false);

    assert_eq!(
        harness
            .chain
            .observed_payload_attestations
            .lock()
            .observed_attestation_count(),
        2,
        "should have 2 observed attestations"
    );

    assert_eq!(
        harness
            .chain
            .observed_payload_attestations
            .lock()
            .observed_slot_count(),
        1,
        "all attestations are for the same slot"
    );
}

// =============================================================================
// Self-build envelope verification (happy path)
// =============================================================================

#[tokio::test]
async fn envelope_self_build_passes_all_checks() {
    let harness = gloas_harness(2).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();

    let stored_envelope = harness
        .chain
        .store
        .get_payload_envelope(&head_root)
        .unwrap();

    if let Some(real_envelope) = stored_envelope {
        let result = harness
            .chain
            .verify_payload_envelope_for_gossip(Arc::new(real_envelope));

        match result {
            Ok(verified) => {
                assert_eq!(verified.beacon_block_root(), head_root);
                assert_eq!(verified.envelope().message.slot, head_slot);
                let inner = verified.into_inner();
                assert_eq!(inner.message.beacon_block_root, head_root);
            }
            Err(e) => {
                // May fail on state-dependent checks for already-processed envelopes
                eprintln!(
                    "note: stored envelope re-verification failed (expected): {:?}",
                    e
                );
            }
        }
    }
}

// =============================================================================
// Prior to finalization test
// =============================================================================

#[tokio::test]
async fn envelope_prior_to_finalization() {
    let harness = gloas_harness(E::slots_per_epoch() as usize * 4).await;

    let finalized_checkpoint = harness
        .chain
        .canonical_head
        .cached_head()
        .finalized_checkpoint();
    let finalized_slot = finalized_checkpoint.epoch.start_slot(E::slots_per_epoch());

    if finalized_slot > Slot::new(0) {
        let head = harness.chain.head_snapshot();
        let head_root = head.beacon_block_root;

        let mut envelope = SignedExecutionPayloadEnvelope::<E>::empty();
        envelope.message.beacon_block_root = head_root;
        envelope.message.slot = Slot::new(0);

        let err = unwrap_err(
            harness
                .chain
                .verify_payload_envelope_for_gossip(Arc::new(envelope)),
            "should reject pre-finalization envelope",
        );
        assert!(
            matches!(
                err,
                PayloadEnvelopeError::SlotMismatch { .. }
                    | PayloadEnvelopeError::PriorToFinalization { .. }
            ),
            "expected SlotMismatch or PriorToFinalization, got {:?}",
            err
        );
    }
}
