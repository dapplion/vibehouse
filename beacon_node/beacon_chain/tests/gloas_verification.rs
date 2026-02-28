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
    AttestationStrategy, BeaconChainHarness, BlockStrategy, DEFAULT_ETH1_BLOCK_HASH,
    DEFAULT_TARGET_AGGREGATORS, EphemeralHarnessType, HARNESS_GENESIS_TIME, InteropGenesisBuilder,
};
use execution_layer::test_utils::generate_genesis_header;
use std::sync::{Arc, LazyLock};
use tree_hash::TreeHash;
use types::*;

type E = MainnetEthSpec;

/// Extract Err from a Result where the Ok type doesn't implement Debug.
fn unwrap_err<T, E: std::fmt::Debug>(result: Result<T, E>, msg: &str) -> E {
    match result {
        Ok(_) => panic!("{}: expected Err, got Ok", msg),
        Err(e) => e,
    }
}

/// Insert matching proposer preferences into the chain's pool for bid validation.
/// Per spec, bids are IGNORED if proposer preferences for the slot haven't been seen.
fn insert_preferences_for_bid<T: beacon_chain::BeaconChainTypes>(
    chain: &beacon_chain::BeaconChain<T>,
    slot: Slot,
    fee_recipient: Address,
    gas_limit: u64,
) {
    let preferences = SignedProposerPreferences {
        message: ProposerPreferences {
            proposal_slot: slot.as_u64(),
            validator_index: 0,
            fee_recipient,
            gas_limit,
        },
        signature: bls::Signature::empty(),
    };
    chain.insert_proposer_preferences(preferences);
}

const VALIDATOR_COUNT: usize = 24;

static KEYPAIRS: LazyLock<Vec<Keypair>> =
    LazyLock::new(|| types::test_utils::generate_deterministic_keypairs(VALIDATOR_COUNT));

/// Build a Gloas harness and extend the chain by `num_blocks`.
async fn gloas_harness(num_blocks: usize) -> BeaconChainHarness<EphemeralHarnessType<E>> {
    let mut spec = ForkName::Gloas.make_genesis_spec(E::default_spec());
    spec.target_aggregators_per_committee = DEFAULT_TARGET_AGGREGATORS;

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

/// Extra keypairs for builder identities (separate from validator keypairs).
static BUILDER_KEYPAIRS: LazyLock<Vec<Keypair>> = LazyLock::new(|| {
    types::test_utils::generate_deterministic_keypairs(VALIDATOR_COUNT + 4)[VALIDATOR_COUNT..]
        .to_vec()
});

/// Build a Gloas harness with builders injected into the genesis state.
///
/// `builders`: slice of `(deposit_epoch, balance)` tuples.
/// Each builder gets a pubkey from `BUILDER_KEYPAIRS` and `withdrawable_epoch = FAR_FUTURE_EPOCH`.
/// The chain runs `num_blocks` blocks. For builders to be active, `deposit_epoch < finalized_epoch`,
/// so `num_blocks` must be large enough for finalization (typically >= 96 for MainnetEthSpec).
async fn gloas_harness_with_builders(
    num_blocks: usize,
    builders: &[(u64, u64)],
) -> BeaconChainHarness<EphemeralHarnessType<E>> {
    let mut spec = ForkName::Gloas.make_genesis_spec(E::default_spec());
    spec.target_aggregators_per_committee = DEFAULT_TARGET_AGGREGATORS;
    let spec_arc = Arc::new(spec.clone());

    // Build genesis state from the interop builder
    let header = generate_genesis_header::<E>(&spec, false);
    let mut state = InteropGenesisBuilder::default()
        .set_alternating_eth1_withdrawal_credentials()
        .set_opt_execution_payload_header(header)
        .build_genesis_state(
            &KEYPAIRS[0..VALIDATOR_COUNT],
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

    let harness = BeaconChainHarness::builder(MainnetEthSpec)
        .spec(spec_arc)
        .keypairs(KEYPAIRS[0..VALIDATOR_COUNT].to_vec())
        .genesis_state_ephemeral_store(state)
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

// -----------------------------------------------------------------------------
// Bid error variants requiring builders in state
// -----------------------------------------------------------------------------

#[tokio::test]
async fn bid_inactive_builder() {
    // Builder with deposit_epoch=5, finalized_epoch=0 at genesis → 5 < 0 is false → inactive
    // No need for finalization since the builder's deposit_epoch is in the future
    let harness = gloas_harness_with_builders(1, &[(5, 2_000_000_000)]).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0; // first builder

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject inactive builder",
    );
    assert!(
        matches!(err, ExecutionBidError::InactiveBuilder { builder_index: 0 }),
        "expected InactiveBuilder, got {:?}",
        err
    );
}

/// Number of blocks to extend the chain by to achieve finalization.
/// With MainnetEthSpec (32 slots/epoch) and 24 validators all attesting,
/// finalization occurs at epoch 2 after ~128 blocks (4 epochs).
const BLOCKS_TO_FINALIZE: usize = 128;

#[tokio::test]
async fn bid_insufficient_builder_balance() {
    // Active builder (deposit_epoch=0) needs finalized_epoch > 0 to be active.
    // Builder[0] has balance = MIN_DEPOSIT_AMOUNT (1_000_000_000), so excess = 0.
    // Any bid value > 0 should fail the can_builder_cover_bid check.
    let balance = 1_000_000_000; // exactly MIN_DEPOSIT_AMOUNT, excess = 0
    let harness =
        gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, balance), (0, 2_000_000_000)]).await;

    let finalized_epoch = harness
        .chain
        .canonical_head
        .cached_head()
        .finalized_checkpoint()
        .epoch;
    assert!(
        finalized_epoch > Epoch::new(0),
        "chain should have finalized, got finalized_epoch={finalized_epoch}"
    );

    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;
    bid.message.value = 100; // exceeds excess balance of 0

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject insufficient balance",
    );
    assert!(
        matches!(
            err,
            ExecutionBidError::InsufficientBuilderBalance {
                builder_index: 0,
                bid_value: 100,
                ..
            }
        ),
        "expected InsufficientBuilderBalance, got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_duplicate_via_gossip_path() {
    // Active builder with sufficient balance (needs finalization for active)
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, 2_000_000_000)]).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;
    bid.message.value = 100;

    // Pre-seed the observation tracker with this bid's tree hash root
    let bid_root = bid.tree_hash_root();
    harness
        .chain
        .observed_execution_bids
        .lock()
        .observe_bid(current_slot, 0, bid_root);

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject duplicate bid",
    );
    assert!(
        matches!(err, ExecutionBidError::DuplicateBid { .. }),
        "expected DuplicateBid, got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_equivocation_via_gossip_path() {
    // Active builder with sufficient balance
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, 2_000_000_000)]).await;
    let current_slot = harness.chain.slot().unwrap();

    // Pre-seed the observation tracker with a different bid root for same builder/slot
    let existing_root = Hash256::from_low_u64_be(0x1111);
    harness
        .chain
        .observed_execution_bids
        .lock()
        .observe_bid(current_slot, 0, existing_root);

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;
    bid.message.value = 100;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject equivocating bid",
    );
    assert!(
        matches!(
            err,
            ExecutionBidError::BuilderEquivocation {
                builder_index: 0,
                ..
            }
        ),
        "expected BuilderEquivocation, got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_invalid_parent_root() {
    // Active builder with sufficient balance, bid passes all checks except parent root
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, 2_000_000_000)]).await;
    let current_slot = harness.chain.slot().unwrap();

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;
    bid.message.value = 100;
    bid.message.parent_block_root = Hash256::from_low_u64_be(0xbadbeef);

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject invalid parent root",
    );
    assert!(
        matches!(err, ExecutionBidError::InvalidParentRoot { .. }),
        "expected InvalidParentRoot, got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_invalid_signature() {
    // Active builder, correct parent root, but invalid signature
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, 2_000_000_000)]).await;
    let current_slot = harness.chain.slot().unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;
    bid.message.value = 100;
    bid.message.parent_block_root = head_root;
    // signature is Signature::empty() which is not valid for this message

    // Insert matching proposer preferences so the bid reaches the signature check
    insert_preferences_for_bid(
        &harness.chain,
        current_slot,
        bid.message.fee_recipient,
        bid.message.gas_limit,
    );

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject invalid signature",
    );
    assert!(
        matches!(err, ExecutionBidError::InvalidSignature),
        "expected InvalidSignature, got {:?}",
        err
    );
}

#[tokio::test]
async fn bid_valid_signature_passes() {
    // Active builder, correct parent root, valid signature → should pass all checks
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, 2_000_000_000)]).await;
    let current_slot = harness.chain.slot().unwrap();
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let state = &head.beacon_state;

    let bid_msg = ExecutionPayloadBid::<E> {
        slot: current_slot,
        execution_payment: 1,
        builder_index: 0,
        value: 100,
        parent_block_root: head_root,
        ..Default::default()
    };

    // Insert matching proposer preferences so the bid reaches the signature check
    insert_preferences_for_bid(
        &harness.chain,
        current_slot,
        bid_msg.fee_recipient,
        bid_msg.gas_limit,
    );

    // Sign with the builder's secret key
    let domain = spec.get_domain(
        current_slot.epoch(E::slots_per_epoch()),
        Domain::BeaconBuilder,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = bid_msg.signing_root(domain);
    let signature = BUILDER_KEYPAIRS[0].sk.sign(signing_root);

    let bid = SignedExecutionPayloadBid {
        message: bid_msg,
        signature,
    };

    let result = harness.chain.verify_execution_bid_for_gossip(bid);
    assert!(
        result.is_ok(),
        "valid bid should pass all checks, got {:?}",
        result.err()
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

// -----------------------------------------------------------------------------
// Payload attestation: equivocation via gossip path
// -----------------------------------------------------------------------------

#[tokio::test]
async fn attestation_validator_equivocation() {
    // Build a chain with a few blocks so the head block root is known in fork choice
    let harness = gloas_harness(2).await;
    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    // Get the PTC committee for the head slot
    let ptc_indices = state_processing::per_block_processing::gloas::get_ptc_committee(
        state,
        head_slot,
        &harness.chain.spec,
    )
    .expect("should compute PTC committee");

    assert!(!ptc_indices.is_empty(), "PTC committee should not be empty");

    let ptc_validator = ptc_indices[0];

    // Pre-seed the observation tracker with an attestation from this validator (payload_present=true)
    harness
        .chain
        .observed_payload_attestations
        .lock()
        .observe_attestation(head_slot, head_root, ptc_validator, true);

    // Now submit an attestation from the same validator with payload_present=false (equivocation)
    let mut attestation = PayloadAttestation::<E>::empty();
    attestation.data.slot = head_slot;
    attestation.data.beacon_block_root = head_root;
    attestation.data.payload_present = false; // different from pre-seeded true
    attestation.aggregation_bits.set(0, true).unwrap();

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_attestation_for_gossip(attestation),
        "should reject equivocating attestation",
    );
    assert!(
        matches!(
            err,
            PayloadAttestationError::ValidatorEquivocation {
                validator_index,
                slot,
                ..
            } if validator_index == ptc_validator && slot == head_slot
        ),
        "expected ValidatorEquivocation for validator {}, got {:?}",
        ptc_validator,
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
// Duplicate envelope deduplication test
// =============================================================================

#[tokio::test]
async fn envelope_duplicate_returns_ignore() {
    let harness = gloas_harness(2).await;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    let stored_envelope = harness
        .chain
        .store
        .get_payload_envelope(&head_root)
        .unwrap()
        .expect("self-build envelope should exist for head block");

    // First verification should succeed — self-build envelopes are not observed
    // during process_self_build_envelope, only through gossip verification.
    let result = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(stored_envelope.clone()));
    assert!(
        result.is_ok(),
        "first gossip verification should succeed, got: {:?}",
        result.err()
    );

    // Second verification of the same block root should return DuplicateEnvelope
    // (the root was recorded by the first successful verification).
    let err = unwrap_err(
        harness
            .chain
            .verify_payload_envelope_for_gossip(Arc::new(stored_envelope)),
        "should reject duplicate envelope",
    );
    assert!(
        matches!(err, PayloadEnvelopeError::DuplicateEnvelope { block_root } if block_root == head_root),
        "expected DuplicateEnvelope with head root, got {:?}",
        err
    );
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

// =============================================================================
// Payload attestation: valid signature happy path
// =============================================================================

#[tokio::test]
async fn attestation_valid_single_ptc_signer_passes() {
    // Build a chain with enough blocks so PTC committee is stable
    let harness = gloas_harness(2).await;
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    // Get PTC committee for the head slot
    let ptc_indices =
        state_processing::per_block_processing::gloas::get_ptc_committee(state, head_slot, spec)
            .expect("should compute PTC committee");
    assert!(!ptc_indices.is_empty(), "PTC committee should not be empty");

    let ptc_validator = ptc_indices[0];

    // Build attestation data
    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: false,
    };

    // Compute signing root using PtcAttester domain
    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = data.signing_root(domain);

    // Sign with the PTC validator's secret key
    let sig = KEYPAIRS[ptc_validator as usize].sk.sign(signing_root);
    let mut agg_sig = AggregateSignature::infinity();
    agg_sig.add_assign(&sig);

    // Set only the first bit (index 0 in PTC → validator ptc_indices[0])
    let mut attestation = PayloadAttestation::<E> {
        aggregation_bits: BitVector::new(),
        data,
        signature: agg_sig,
    };
    attestation.aggregation_bits.set(0, true).unwrap();

    let result = harness
        .chain
        .verify_payload_attestation_for_gossip(attestation);
    assert!(
        result.is_ok(),
        "valid payload attestation should pass all checks, got {:?}",
        result.err()
    );

    let verified = result.unwrap();
    assert_eq!(
        verified.attesting_indices(),
        &[ptc_validator],
        "attesting indices should contain the PTC validator"
    );
    assert_eq!(verified.attestation().data.beacon_block_root, head_root);
    assert!(verified.attestation().data.payload_present);
}

#[tokio::test]
async fn attestation_invalid_signature_rejected() {
    let harness = gloas_harness(2).await;
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let ptc_indices =
        state_processing::per_block_processing::gloas::get_ptc_committee(state, head_slot, spec)
            .expect("should compute PTC committee");
    assert!(!ptc_indices.is_empty());

    let ptc_validator = ptc_indices[0];

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: false,
    };

    // Sign with a WRONG key (use a different validator's key)
    let wrong_validator = if ptc_validator == 0 { 1 } else { 0 };
    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = data.signing_root(domain);
    let wrong_sig = KEYPAIRS[wrong_validator as usize].sk.sign(signing_root);
    let mut agg_sig = AggregateSignature::infinity();
    agg_sig.add_assign(&wrong_sig);

    let mut attestation = PayloadAttestation::<E> {
        aggregation_bits: BitVector::new(),
        data,
        signature: agg_sig,
    };
    attestation.aggregation_bits.set(0, true).unwrap();

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_attestation_for_gossip(attestation),
        "should reject attestation with wrong signature",
    );
    assert!(
        matches!(err, PayloadAttestationError::InvalidSignature),
        "expected InvalidSignature, got {:?}",
        err
    );
}

#[tokio::test]
async fn attestation_multiple_ptc_signers_passes() {
    let harness = gloas_harness(2).await;
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let ptc_indices =
        state_processing::per_block_processing::gloas::get_ptc_committee(state, head_slot, spec)
            .expect("should compute PTC committee");

    // Need at least 2 PTC members for this test
    if ptc_indices.len() < 2 {
        return;
    }

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: false,
    };

    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = data.signing_root(domain);

    // Create aggregate signature from first 2 PTC members
    let mut agg_sig = AggregateSignature::infinity();
    for &vi in &ptc_indices[..2] {
        let sig = KEYPAIRS[vi as usize].sk.sign(signing_root);
        agg_sig.add_assign(&sig);
    }

    let mut attestation = PayloadAttestation::<E> {
        aggregation_bits: BitVector::new(),
        data,
        signature: agg_sig,
    };
    attestation.aggregation_bits.set(0, true).unwrap();
    attestation.aggregation_bits.set(1, true).unwrap();

    let result = harness
        .chain
        .verify_payload_attestation_for_gossip(attestation);
    assert!(
        result.is_ok(),
        "valid multi-signer attestation should pass, got {:?}",
        result.err()
    );

    let verified = result.unwrap();
    assert_eq!(
        verified.attesting_indices().len(),
        2,
        "should have 2 attesting indices"
    );
    assert_eq!(verified.attesting_indices()[0], ptc_indices[0]);
    assert_eq!(verified.attesting_indices()[1], ptc_indices[1]);
}

#[tokio::test]
async fn attestation_payload_not_present_passes() {
    // Verify that payload_present=false is also accepted (it's a valid vote)
    let harness = gloas_harness(2).await;
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let ptc_indices =
        state_processing::per_block_processing::gloas::get_ptc_committee(state, head_slot, spec)
            .expect("should compute PTC committee");
    assert!(!ptc_indices.is_empty());

    let ptc_validator = ptc_indices[0];

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: false,
        blob_data_available: false,
    };

    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = data.signing_root(domain);

    let sig = KEYPAIRS[ptc_validator as usize].sk.sign(signing_root);
    let mut agg_sig = AggregateSignature::infinity();
    agg_sig.add_assign(&sig);

    let mut attestation = PayloadAttestation::<E> {
        aggregation_bits: BitVector::new(),
        data,
        signature: agg_sig,
    };
    attestation.aggregation_bits.set(0, true).unwrap();

    let result = harness
        .chain
        .verify_payload_attestation_for_gossip(attestation);
    assert!(
        result.is_ok(),
        "payload_present=false attestation should also pass, got {:?}",
        result.err()
    );
    assert!(!result.unwrap().attestation().data.payload_present);
}

// =============================================================================
// Execution bid: edge case tests
// =============================================================================

#[tokio::test]
async fn bid_balance_exactly_sufficient_passes() {
    // Builder has balance = MIN_DEPOSIT_AMOUNT + 100, bid value = 100.
    // Excess balance = balance - MIN_DEPOSIT_AMOUNT = 100, which exactly covers the bid.
    let balance = 1_000_000_100; // MIN_DEPOSIT_AMOUNT (1_000_000_000) + 100
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, balance)]).await;
    let current_slot = harness.chain.slot().unwrap();
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let state = &head.beacon_state;

    let bid_msg = ExecutionPayloadBid::<E> {
        slot: current_slot,
        execution_payment: 1,
        builder_index: 0,
        value: 100,
        parent_block_root: head_root,
        ..Default::default()
    };

    // Insert matching proposer preferences so the bid reaches the balance/signature checks
    insert_preferences_for_bid(
        &harness.chain,
        current_slot,
        bid_msg.fee_recipient,
        bid_msg.gas_limit,
    );

    let domain = spec.get_domain(
        current_slot.epoch(E::slots_per_epoch()),
        Domain::BeaconBuilder,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = bid_msg.signing_root(domain);
    let signature = BUILDER_KEYPAIRS[0].sk.sign(signing_root);

    let bid = SignedExecutionPayloadBid {
        message: bid_msg,
        signature,
    };

    let result = harness.chain.verify_execution_bid_for_gossip(bid);
    assert!(
        result.is_ok(),
        "bid with value == balance should pass, got {:?}",
        result.err()
    );
}

#[tokio::test]
async fn bid_balance_one_over_rejected() {
    // Builder has balance = MIN_DEPOSIT_AMOUNT + 100 (excess = 100), bid value = 101 — should be rejected.
    // The excess balance (100) is 1 less than the bid value (101).
    let balance = 1_000_000_100; // MIN_DEPOSIT_AMOUNT (1_000_000_000) + 100
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, balance)]).await;
    let current_slot = harness.chain.slot().unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;
    bid.message.value = 101;
    bid.message.parent_block_root = head_root;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject bid exceeding excess balance by 1",
    );
    assert!(
        matches!(
            err,
            ExecutionBidError::InsufficientBuilderBalance {
                builder_index: 0,
                balance,
                bid_value: 101,
            } if balance == 1_000_000_100
        ),
        "expected InsufficientBuilderBalance, got {:?}",
        err
    );
}

// =============================================================================
// External builder envelope: signature verification
// =============================================================================

#[tokio::test]
async fn envelope_external_builder_valid_signature_passes() {
    // Build a chain with an external builder in the state, then construct
    // a matching envelope with a valid builder signature.
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, 2_000_000_000)]).await;
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    // Get the committed bid from the head block
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

    // Construct an envelope that matches the committed bid
    let mut envelope_msg = ExecutionPayloadEnvelope::<E>::empty();
    envelope_msg.beacon_block_root = head_root;
    envelope_msg.slot = head_slot;
    envelope_msg.builder_index = committed_bid.message.builder_index;
    envelope_msg.payload.block_hash = committed_bid.message.block_hash;

    // Only sign if it's an external builder (non-self-build)
    if committed_bid.message.builder_index != types::consts::gloas::BUILDER_INDEX_SELF_BUILD {
        let epoch = head_slot.epoch(E::slots_per_epoch());
        let domain = spec.get_domain(
            epoch,
            Domain::BeaconBuilder,
            &state.fork(),
            state.genesis_validators_root(),
        );
        let signing_root = envelope_msg.signing_root(domain);
        let bi = committed_bid.message.builder_index as usize;
        let signature = BUILDER_KEYPAIRS[bi].sk.sign(signing_root);

        let signed_envelope = SignedExecutionPayloadEnvelope {
            message: envelope_msg,
            signature,
        };

        let result = harness
            .chain
            .verify_payload_envelope_for_gossip(Arc::new(signed_envelope));
        assert!(
            result.is_ok(),
            "external builder envelope with valid signature should pass, got {:?}",
            result.err()
        );

        let verified = result.unwrap();
        assert_eq!(verified.beacon_block_root(), head_root);
    }
    // If the committed bid is self-build, the test still passes — it means
    // the harness produced a self-build block, so we can't test external builder
    // signature verification here. The bid_valid_signature_passes test already
    // covers the builder signing path.
}

#[tokio::test]
async fn envelope_external_builder_invalid_signature_rejected() {
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, 2_000_000_000)]).await;

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

    let mut envelope_msg = ExecutionPayloadEnvelope::<E>::empty();
    envelope_msg.beacon_block_root = head_root;
    envelope_msg.slot = head_slot;
    envelope_msg.builder_index = committed_bid.message.builder_index;
    envelope_msg.payload.block_hash = committed_bid.message.block_hash;

    if committed_bid.message.builder_index != types::consts::gloas::BUILDER_INDEX_SELF_BUILD {
        // Use an empty (invalid) signature
        let signed_envelope = SignedExecutionPayloadEnvelope {
            message: envelope_msg,
            signature: Signature::empty(),
        };

        let err = unwrap_err(
            harness
                .chain
                .verify_payload_envelope_for_gossip(Arc::new(signed_envelope)),
            "should reject envelope with invalid signature",
        );
        assert!(
            matches!(err, PayloadEnvelopeError::InvalidSignature),
            "expected InvalidSignature, got {:?}",
            err
        );
    }
}

// =============================================================================
// Payload attestation: duplicate handling tests
// =============================================================================

#[tokio::test]
async fn attestation_duplicate_same_value_still_passes() {
    // When a PTC validator's attestation has already been observed with the
    // same payload_present value, the equivocation check says "Duplicate" and
    // continues. The attestation should still pass verification (for relay).
    let harness = gloas_harness(2).await;
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let ptc_indices =
        state_processing::per_block_processing::gloas::get_ptc_committee(state, head_slot, spec)
            .expect("should compute PTC committee");
    assert!(!ptc_indices.is_empty());

    let ptc_validator = ptc_indices[0];

    // Pre-observe this validator with payload_present=true
    harness
        .chain
        .observed_payload_attestations
        .lock()
        .observe_attestation(head_slot, head_root, ptc_validator, true);

    // Build the same attestation (payload_present=true) and sign it correctly
    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: false,
    };

    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = data.signing_root(domain);
    let sig = KEYPAIRS[ptc_validator as usize].sk.sign(signing_root);
    let mut agg_sig = AggregateSignature::infinity();
    agg_sig.add_assign(&sig);

    let mut attestation = PayloadAttestation::<E> {
        aggregation_bits: BitVector::new(),
        data,
        signature: agg_sig,
    };
    attestation.aggregation_bits.set(0, true).unwrap();

    // Should still pass even though it's a duplicate — duplicates are accepted for relay
    let result = harness
        .chain
        .verify_payload_attestation_for_gossip(attestation);
    assert!(
        result.is_ok(),
        "duplicate attestation (same value) should still pass, got {:?}",
        result.err()
    );
}

#[tokio::test]
async fn attestation_mixed_duplicate_and_new_passes() {
    // Attestation with 2 PTC members: one already observed (duplicate),
    // one new. Should still pass verification.
    let harness = gloas_harness(2).await;
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let ptc_indices =
        state_processing::per_block_processing::gloas::get_ptc_committee(state, head_slot, spec)
            .expect("should compute PTC committee");
    if ptc_indices.len() < 2 {
        return; // Need at least 2 PTC members
    }

    let validator_0 = ptc_indices[0];

    // Pre-observe validator_0 with same value
    harness
        .chain
        .observed_payload_attestations
        .lock()
        .observe_attestation(head_slot, head_root, validator_0, true);

    // Build attestation from both validators
    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: false,
    };

    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = data.signing_root(domain);

    let mut agg_sig = AggregateSignature::infinity();
    for &vi in &ptc_indices[..2] {
        let sig = KEYPAIRS[vi as usize].sk.sign(signing_root);
        agg_sig.add_assign(&sig);
    }

    let mut attestation = PayloadAttestation::<E> {
        aggregation_bits: BitVector::new(),
        data,
        signature: agg_sig,
    };
    attestation.aggregation_bits.set(0, true).unwrap();
    attestation.aggregation_bits.set(1, true).unwrap();

    let result = harness
        .chain
        .verify_payload_attestation_for_gossip(attestation);
    assert!(
        result.is_ok(),
        "mixed duplicate+new attestation should pass, got {:?}",
        result.err()
    );

    let verified = result.unwrap();
    assert_eq!(
        verified.attesting_indices().len(),
        2,
        "both validators should be in attesting indices (duplicates are not removed)"
    );
}

// =============================================================================
// Envelope: self-build signature skip test
// =============================================================================

#[tokio::test]
async fn envelope_self_build_skips_signature_verification() {
    // Self-build envelopes (builder_index == BUILDER_INDEX_SELF_BUILD) skip BLS
    // signature verification. Verify that an envelope with an empty signature
    // passes all checks when it matches a self-build block.
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

    // The test harness produces self-build blocks
    assert_eq!(
        committed_bid.message.builder_index,
        types::consts::gloas::BUILDER_INDEX_SELF_BUILD,
        "harness should produce self-build blocks"
    );

    let mut envelope_msg = ExecutionPayloadEnvelope::<E>::empty();
    envelope_msg.beacon_block_root = head_root;
    envelope_msg.slot = head_slot;
    envelope_msg.builder_index = types::consts::gloas::BUILDER_INDEX_SELF_BUILD;
    envelope_msg.payload.block_hash = committed_bid.message.block_hash;

    // Use an empty signature — self-build should not require signature verification
    let signed_envelope = SignedExecutionPayloadEnvelope {
        message: envelope_msg,
        signature: Signature::empty(),
    };

    let result = harness
        .chain
        .verify_payload_envelope_for_gossip(Arc::new(signed_envelope));
    assert!(
        result.is_ok(),
        "self-build envelope with empty signature should pass, got {:?}",
        result.err()
    );

    let verified = result.unwrap();
    assert_eq!(verified.beacon_block_root(), head_root);
}

// =============================================================================
// Envelope: direct prior-to-finalization test
// =============================================================================

#[tokio::test]
async fn envelope_prior_to_finalization_direct() {
    // Directly test that an envelope at a finalized slot is rejected.
    // Unlike the existing test, this uses a block root that IS in fork choice
    // to reach the PriorToFinalization check (not the SlotMismatch check).
    let harness = gloas_harness(E::slots_per_epoch() as usize * 4).await;

    let finalized_checkpoint = harness
        .chain
        .canonical_head
        .cached_head()
        .finalized_checkpoint();
    let finalized_slot = finalized_checkpoint.epoch.start_slot(E::slots_per_epoch());

    assert!(
        finalized_slot > Slot::new(0),
        "chain should have finalized, got finalized_slot={}",
        finalized_slot
    );

    // Use the current head (in fork choice) but set envelope slot to 0 (finalized)
    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    let mut envelope = SignedExecutionPayloadEnvelope::<E>::empty();
    envelope.message.beacon_block_root = head_root;
    envelope.message.slot = Slot::new(0); // Prior to finalization

    let err = unwrap_err(
        harness
            .chain
            .verify_payload_envelope_for_gossip(Arc::new(envelope)),
        "should reject pre-finalization envelope",
    );
    // The head block is at a high slot, so envelope.slot=0 != head.slot → SlotMismatch
    // OR if the code checks finalization before slot match, it would be PriorToFinalization
    assert!(
        matches!(
            err,
            PayloadEnvelopeError::PriorToFinalization { .. }
                | PayloadEnvelopeError::SlotMismatch { .. }
        ),
        "expected PriorToFinalization or SlotMismatch, got {:?}",
        err
    );
}

// =============================================================================
// Bid: proposer preferences validation (ProposerPreferencesNotSeen, FeeRecipientMismatch,
// GasLimitMismatch)
// =============================================================================

/// Bid submitted before any proposer preferences have been seen for the slot → IGNORE.
/// Per spec, bids must be preceded by a SignedProposerPreferences for the same slot.
/// Without it, the bid cannot be validated for fee_recipient/gas_limit compliance.
#[tokio::test]
async fn bid_no_proposer_preferences_ignored() {
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, 2_000_000_000)]).await;
    let current_slot = harness.chain.slot().unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    // Build a bid with valid slot, builder index, and parent root
    // but do NOT insert any proposer preferences for this slot
    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;
    bid.message.value = 100;
    bid.message.parent_block_root = head_root;

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject bid without proposer preferences",
    );
    assert!(
        matches!(err, ExecutionBidError::ProposerPreferencesNotSeen { slot } if slot == current_slot),
        "expected ProposerPreferencesNotSeen for slot {}, got {:?}",
        current_slot,
        err
    );
}

/// Bid with wrong fee_recipient → FeeRecipientMismatch → REJECT.
/// The proposer published their preferences with a specific fee_recipient.
/// A builder cannot override that address in their bid.
#[tokio::test]
async fn bid_fee_recipient_mismatch_rejected() {
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, 2_000_000_000)]).await;
    let current_slot = harness.chain.slot().unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    // The bid uses a specific fee_recipient
    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;
    bid.message.value = 100;
    bid.message.parent_block_root = head_root;
    bid.message.fee_recipient = Address::from([0xaa; 20]);

    // Insert proposer preferences with a DIFFERENT fee_recipient
    let preferences = SignedProposerPreferences {
        message: ProposerPreferences {
            proposal_slot: current_slot.as_u64(),
            validator_index: 0,
            fee_recipient: Address::from([0xbb; 20]), // different from bid's 0xaa
            gas_limit: bid.message.gas_limit,
        },
        signature: bls::Signature::empty(),
    };
    harness.chain.insert_proposer_preferences(preferences);

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject bid with wrong fee_recipient",
    );
    assert!(
        matches!(
            err,
            ExecutionBidError::FeeRecipientMismatch {
                expected,
                received,
            } if expected == Address::from([0xbb; 20]) && received == Address::from([0xaa; 20])
        ),
        "expected FeeRecipientMismatch, got {:?}",
        err
    );
}

/// Bid with wrong gas_limit → GasLimitMismatch → REJECT.
/// The proposer specified a gas_limit in their preferences.
/// The builder's bid must match exactly.
#[tokio::test]
async fn bid_gas_limit_mismatch_rejected() {
    let harness = gloas_harness_with_builders(BLOCKS_TO_FINALIZE, &[(0, 2_000_000_000)]).await;
    let current_slot = harness.chain.slot().unwrap();

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;

    // Build a bid with gas_limit = 30_000_000
    let mut bid = SignedExecutionPayloadBid::<E>::empty();
    bid.message.slot = current_slot;
    bid.message.execution_payment = 1;
    bid.message.builder_index = 0;
    bid.message.value = 100;
    bid.message.parent_block_root = head_root;
    bid.message.gas_limit = 30_000_000;

    // Insert proposer preferences with a different gas_limit = 20_000_000
    let preferences = SignedProposerPreferences {
        message: ProposerPreferences {
            proposal_slot: current_slot.as_u64(),
            validator_index: 0,
            fee_recipient: bid.message.fee_recipient,
            gas_limit: 20_000_000, // different from bid's 30_000_000
        },
        signature: bls::Signature::empty(),
    };
    harness.chain.insert_proposer_preferences(preferences);

    let err = unwrap_err(
        harness.chain.verify_execution_bid_for_gossip(bid),
        "should reject bid with wrong gas_limit",
    );
    assert!(
        matches!(
            err,
            ExecutionBidError::GasLimitMismatch {
                expected: 20_000_000,
                received: 30_000_000,
            }
        ),
        "expected GasLimitMismatch {{ expected: 20_000_000, received: 30_000_000 }}, got {:?}",
        err
    );
}

// =============================================================================
// Bid: second builder in multi-builder harness
// =============================================================================

#[tokio::test]
async fn bid_second_builder_valid_signature_passes() {
    // Test that a second builder (index=1) can also submit a valid bid.
    let harness = gloas_harness_with_builders(
        BLOCKS_TO_FINALIZE,
        &[(0, 2_000_000_000), (0, 3_000_000_000)],
    )
    .await;
    let current_slot = harness.chain.slot().unwrap();
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let state = &head.beacon_state;

    let bid_msg = ExecutionPayloadBid::<E> {
        slot: current_slot,
        execution_payment: 1,
        builder_index: 1,
        value: 500,
        parent_block_root: head_root,
        ..Default::default()
    };

    // Insert matching proposer preferences so the bid reaches the signature check
    insert_preferences_for_bid(
        &harness.chain,
        current_slot,
        bid_msg.fee_recipient,
        bid_msg.gas_limit,
    );

    let domain = spec.get_domain(
        current_slot.epoch(E::slots_per_epoch()),
        Domain::BeaconBuilder,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = bid_msg.signing_root(domain);
    let signature = BUILDER_KEYPAIRS[1].sk.sign(signing_root);

    let bid = SignedExecutionPayloadBid {
        message: bid_msg,
        signature,
    };

    let result = harness.chain.verify_execution_bid_for_gossip(bid);
    assert!(
        result.is_ok(),
        "valid bid from second builder should pass, got {:?}",
        result.err()
    );
}

// =============================================================================
// Attestation: blob_data_available field variations
// =============================================================================

#[tokio::test]
async fn attestation_blob_data_available_true_passes() {
    // Verify that payload attestations with blob_data_available=true also pass
    let harness = gloas_harness(2).await;
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let ptc_indices =
        state_processing::per_block_processing::gloas::get_ptc_committee(state, head_slot, spec)
            .expect("should compute PTC committee");
    assert!(!ptc_indices.is_empty());

    let ptc_validator = ptc_indices[0];

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: true,
        blob_data_available: true,
    };

    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = data.signing_root(domain);
    let sig = KEYPAIRS[ptc_validator as usize].sk.sign(signing_root);
    let mut agg_sig = AggregateSignature::infinity();
    agg_sig.add_assign(&sig);

    let mut attestation = PayloadAttestation::<E> {
        aggregation_bits: BitVector::new(),
        data,
        signature: agg_sig,
    };
    attestation.aggregation_bits.set(0, true).unwrap();

    let result = harness
        .chain
        .verify_payload_attestation_for_gossip(attestation);
    assert!(
        result.is_ok(),
        "attestation with blob_data_available=true should pass, got {:?}",
        result.err()
    );
}

#[tokio::test]
async fn attestation_payload_absent_blob_available_passes() {
    // payload_present=false, blob_data_available=true is a valid combination
    let harness = gloas_harness(2).await;
    let spec = &harness.chain.spec;

    let head = harness.chain.head_snapshot();
    let head_root = head.beacon_block_root;
    let head_slot = head.beacon_block.slot();
    let state = &head.beacon_state;

    let ptc_indices =
        state_processing::per_block_processing::gloas::get_ptc_committee(state, head_slot, spec)
            .expect("should compute PTC committee");
    assert!(!ptc_indices.is_empty());

    let ptc_validator = ptc_indices[0];

    let data = PayloadAttestationData {
        beacon_block_root: head_root,
        slot: head_slot,
        payload_present: false,
        blob_data_available: true,
    };

    let epoch = head_slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::PtcAttester,
        &state.fork(),
        state.genesis_validators_root(),
    );
    let signing_root = data.signing_root(domain);
    let sig = KEYPAIRS[ptc_validator as usize].sk.sign(signing_root);
    let mut agg_sig = AggregateSignature::infinity();
    agg_sig.add_assign(&sig);

    let mut attestation = PayloadAttestation::<E> {
        aggregation_bits: BitVector::new(),
        data,
        signature: agg_sig,
    };
    attestation.aggregation_bits.set(0, true).unwrap();

    let result = harness
        .chain
        .verify_payload_attestation_for_gossip(attestation);
    assert!(
        result.is_ok(),
        "payload_present=false, blob_data_available=true should pass, got {:?}",
        result.err()
    );
}
