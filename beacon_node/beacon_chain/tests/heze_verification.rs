#![cfg(not(debug_assertions))]

//! Integration tests for Heze FOCIL (EIP-7805) beacon chain functions:
//!
//! - `verify_inclusion_list_for_gossip` — gossip validation (7 checks)
//! - `import_inclusion_list` — import into InclusionListStore
//! - `get_inclusion_lists_by_committee_indices` — query cached ILs by committee position
//! - `check_inclusion_list_satisfaction` — envelope IL satisfaction check
//! - `compute_inclusion_list_bits_for_slot` — bit computation for block production
//! - `get_best_execution_bid` — Heze bid filtering by inclusion_list_bits

use beacon_chain::heze_verification::InclusionListError;
use beacon_chain::test_utils::{
    AttestationStrategy, BeaconChainHarness, BlockStrategy, EphemeralHarnessType,
};
use state_processing::per_block_processing::heze::get_inclusion_list_committee;
use std::sync::{Arc, LazyLock};
use tree_hash::TreeHash;
use types::*;

type E = MainnetEthSpec;

const VALIDATOR_COUNT: usize = 256;

static KEYPAIRS: LazyLock<Vec<Keypair>> =
    LazyLock::new(|| types::test_utils::generate_deterministic_keypairs(VALIDATOR_COUNT));

/// Build a Heze harness at genesis and extend the chain by `num_blocks`.
async fn heze_harness(num_blocks: usize) -> BeaconChainHarness<EphemeralHarnessType<E>> {
    let mut spec = ForkName::Heze.make_genesis_spec(E::default_spec());
    spec.target_aggregators_per_committee = 1 << 32; // all validators aggregate

    let harness = BeaconChainHarness::builder(MainnetEthSpec)
        .spec(Arc::new(spec))
        .keypairs(KEYPAIRS[0..VALIDATOR_COUNT].to_vec())
        .fresh_ephemeral_store()
        .mock_execution_layer()
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

/// Get the head state's slot (the slot of the last processed block).
fn head_slot(harness: &BeaconChainHarness<EphemeralHarnessType<E>>) -> Slot {
    harness
        .chain
        .canonical_head
        .cached_head()
        .snapshot
        .beacon_state
        .slot()
}

/// Compute the committee and committee_root for a given slot using the head state.
fn committee_info(
    harness: &BeaconChainHarness<EphemeralHarnessType<E>>,
    slot: Slot,
) -> (Vec<u64>, Hash256) {
    let head = harness.chain.canonical_head.cached_head();
    let state = &head.snapshot.beacon_state;
    let spec = &harness.chain.spec;

    let committee = get_inclusion_list_committee(state, slot, spec).unwrap();
    let committee_fixed: ssz_types::FixedVector<u64, <E as EthSpec>::InclusionListCommitteeSize> =
        ssz_types::FixedVector::new(committee.clone()).unwrap();
    let committee_root = committee_fixed.tree_hash_root();

    (committee, committee_root)
}

/// Create a signed inclusion list (with empty signature — only used for store manipulation).
fn make_signed_il(
    slot: Slot,
    validator_index: u64,
    committee_root: Hash256,
    transactions: Vec<Vec<u8>>,
) -> SignedInclusionList<E> {
    let tx_vecs: Vec<VariableList<u8, <E as EthSpec>::MaxBytesPerTransaction>> = transactions
        .into_iter()
        .map(|tx| VariableList::new(tx).unwrap())
        .collect();
    let tx_list = VariableList::new(tx_vecs).unwrap();

    SignedInclusionList {
        message: InclusionList {
            slot,
            validator_index,
            inclusion_list_committee_root: committee_root,
            transactions: tx_list,
        },
        signature: bls::Signature::empty(),
    }
}

/// Create a properly signed inclusion list using deterministic keypairs (for gossip verification).
fn make_valid_signed_il(
    harness: &BeaconChainHarness<EphemeralHarnessType<E>>,
    slot: Slot,
    validator_index: u64,
    committee_root: Hash256,
    transactions: Vec<Vec<u8>>,
) -> SignedInclusionList<E> {
    let tx_vecs: Vec<VariableList<u8, <E as EthSpec>::MaxBytesPerTransaction>> = transactions
        .into_iter()
        .map(|tx| VariableList::new(tx).unwrap())
        .collect();
    let tx_list = VariableList::new(tx_vecs).unwrap();

    let il = InclusionList {
        slot,
        validator_index,
        inclusion_list_committee_root: committee_root,
        transactions: tx_list,
    };

    let head = harness.chain.canonical_head.cached_head();
    let state = &head.snapshot.beacon_state;
    let spec = &harness.chain.spec;

    let epoch = slot.epoch(E::slots_per_epoch());
    let domain = spec.get_domain(
        epoch,
        Domain::InclusionListCommittee,
        &state.fork(),
        state.genesis_validators_root(),
    );

    let signing_root = SigningData {
        object_root: il.tree_hash_root(),
        domain,
    }
    .tree_hash_root();

    let signature = KEYPAIRS[validator_index as usize].sk.sign(signing_root);

    SignedInclusionList {
        message: il,
        signature,
    }
}

// ===========================================================================
// verify_inclusion_list_for_gossip tests
// ===========================================================================

/// Helper: unwrap Err from gossip verification (Ok variant doesn't impl Debug).
fn unwrap_il_err(
    result: Result<
        beacon_chain::heze_verification::VerifiedInclusionList<EphemeralHarnessType<E>>,
        InclusionListError,
    >,
    msg: &str,
) -> InclusionListError {
    match result {
        Ok(_) => panic!("{msg}: expected Err, got Ok"),
        Err(e) => e,
    }
}

#[tokio::test]
async fn gossip_il_valid_current_slot() {
    let harness = heze_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();
    let (committee, committee_root) = committee_info(&harness, current_slot);

    let signed_il =
        make_valid_signed_il(&harness, current_slot, committee[0], committee_root, vec![]);

    let result = harness.chain.verify_inclusion_list_for_gossip(signed_il);
    assert!(result.is_ok(), "valid IL at current slot should pass");
}

#[tokio::test]
async fn gossip_il_valid_import_roundtrip() {
    let harness = heze_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();
    let (committee, committee_root) = committee_info(&harness, current_slot);

    let signed_il = make_valid_signed_il(
        &harness,
        current_slot,
        committee[0],
        committee_root,
        vec![vec![0xaa, 0xbb]],
    );

    let verified = harness
        .chain
        .verify_inclusion_list_for_gossip(signed_il)
        .unwrap_or_else(|e| panic!("should verify: {e:?}"));

    // Import and verify it shows up in the store
    harness.chain.import_inclusion_list(verified);

    let result = harness
        .chain
        .get_inclusion_lists_by_committee_indices(current_slot, &[0])
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].message.validator_index, committee[0]);
}

#[tokio::test]
async fn gossip_il_reject_wrong_slot() {
    let harness = heze_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();
    let far_future_slot = current_slot + 10;
    let (committee, committee_root) = committee_info(&harness, current_slot);

    let signed_il = make_valid_signed_il(
        &harness,
        far_future_slot,
        committee[0],
        committee_root,
        vec![],
    );

    let err = unwrap_il_err(
        harness.chain.verify_inclusion_list_for_gossip(signed_il),
        "should reject future slot",
    );
    assert!(
        matches!(err, InclusionListError::SlotNotCurrentOrPrevious { .. }),
        "expected SlotNotCurrentOrPrevious, got {err:?}"
    );
}

#[tokio::test]
async fn gossip_il_reject_transactions_too_large() {
    let harness = heze_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();
    let (committee, committee_root) = committee_info(&harness, current_slot);

    // MAX_BYTES_PER_INCLUSION_LIST = 8192
    let big_tx = vec![0xffu8; 8193];
    let signed_il = make_valid_signed_il(
        &harness,
        current_slot,
        committee[0],
        committee_root,
        vec![big_tx],
    );

    let err = unwrap_il_err(
        harness.chain.verify_inclusion_list_for_gossip(signed_il),
        "should reject oversized transactions",
    );
    assert!(
        matches!(err, InclusionListError::TransactionsTooLarge { .. }),
        "expected TransactionsTooLarge, got {err:?}"
    );
}

#[tokio::test]
async fn gossip_il_reject_not_in_committee() {
    let harness = heze_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();
    let (committee, committee_root) = committee_info(&harness, current_slot);

    // Find a validator NOT in the committee
    let not_in_committee = (0..VALIDATOR_COUNT as u64)
        .find(|idx| !committee.contains(idx))
        .expect("should find validator not in committee");

    let signed_il = make_valid_signed_il(
        &harness,
        current_slot,
        not_in_committee,
        committee_root,
        vec![],
    );

    let err = unwrap_il_err(
        harness.chain.verify_inclusion_list_for_gossip(signed_il),
        "should reject non-committee member",
    );
    assert!(
        matches!(err, InclusionListError::NotInCommittee { .. }),
        "expected NotInCommittee, got {err:?}"
    );
}

#[tokio::test]
async fn gossip_il_reject_invalid_signature() {
    let harness = heze_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();
    let (committee, committee_root) = committee_info(&harness, current_slot);

    // Use empty (invalid) signature
    let signed_il = make_signed_il(current_slot, committee[0], committee_root, vec![]);

    let err = unwrap_il_err(
        harness.chain.verify_inclusion_list_for_gossip(signed_il),
        "should reject invalid signature",
    );
    assert!(
        matches!(err, InclusionListError::InvalidSignature),
        "expected InvalidSignature, got {err:?}"
    );
}

#[tokio::test]
async fn gossip_il_reject_committee_root_mismatch() {
    let harness = heze_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();
    let (committee, _committee_root) = committee_info(&harness, current_slot);

    let wrong_root = Hash256::repeat_byte(0xff);
    let signed_il = make_valid_signed_il(&harness, current_slot, committee[0], wrong_root, vec![]);

    let err = unwrap_il_err(
        harness.chain.verify_inclusion_list_for_gossip(signed_il),
        "should reject wrong committee root",
    );
    assert!(
        matches!(err, InclusionListError::CommitteeRootMismatch { .. }),
        "expected CommitteeRootMismatch, got {err:?}"
    );
}

#[tokio::test]
async fn gossip_il_ignore_duplicate() {
    let harness = heze_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();
    let (committee, committee_root) = committee_info(&harness, current_slot);

    let signed_il =
        make_valid_signed_il(&harness, current_slot, committee[0], committee_root, vec![]);

    // First submission should succeed
    let verified = harness
        .chain
        .verify_inclusion_list_for_gossip(signed_il.clone())
        .unwrap_or_else(|e| panic!("first submission should pass: {e:?}"));
    harness.chain.import_inclusion_list(verified);

    // Second identical submission should be IGNORE (duplicate)
    let err = unwrap_il_err(
        harness.chain.verify_inclusion_list_for_gossip(signed_il),
        "duplicate should be rejected",
    );
    assert!(
        matches!(err, InclusionListError::Duplicate { .. }),
        "expected Duplicate, got {err:?}"
    );
}

#[tokio::test]
async fn gossip_il_ignore_equivocator() {
    let harness = heze_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();
    let (committee, committee_root) = committee_info(&harness, current_slot);

    // Submit two different ILs from same validator (creates equivocation)
    let il_a = make_valid_signed_il(&harness, current_slot, committee[0], committee_root, vec![]);
    let il_b = make_valid_signed_il(
        &harness,
        current_slot,
        committee[0],
        committee_root,
        vec![vec![0x01]],
    );

    let verified_a = harness
        .chain
        .verify_inclusion_list_for_gossip(il_a)
        .unwrap_or_else(|e| panic!("first IL should pass: {e:?}"));
    harness.chain.import_inclusion_list(verified_a);

    let verified_b = harness
        .chain
        .verify_inclusion_list_for_gossip(il_b)
        .unwrap_or_else(|e| panic!("second IL should pass: {e:?}"));
    harness.chain.import_inclusion_list(verified_b);

    // Third IL should be IGNORE (equivocator)
    let il_c = make_valid_signed_il(
        &harness,
        current_slot,
        committee[0],
        committee_root,
        vec![vec![0x02]],
    );
    let err = unwrap_il_err(
        harness.chain.verify_inclusion_list_for_gossip(il_c),
        "third IL from equivocator should be rejected",
    );
    assert!(
        matches!(err, InclusionListError::Equivocator { .. }),
        "expected Equivocator, got {err:?}"
    );
}

// ===========================================================================
// get_inclusion_lists_by_committee_indices tests
// ===========================================================================

#[tokio::test]
async fn get_inclusion_lists_empty_store() {
    let harness = heze_harness(1).await;
    let current_slot = head_slot(&harness);
    let (committee, _) = committee_info(&harness, current_slot);

    // Query all positions — store is empty so should return empty
    let indices: Vec<u64> = (0..committee.len() as u64).collect();
    let result = harness
        .chain
        .get_inclusion_lists_by_committee_indices(current_slot, &indices)
        .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn get_inclusion_lists_returns_cached_ils() {
    let harness = heze_harness(1).await;
    let current_slot = head_slot(&harness);
    let (committee, committee_root) = committee_info(&harness, current_slot);

    // Insert ILs for first two committee members
    let il0 = make_signed_il(current_slot, committee[0], committee_root, vec![vec![1, 2]]);
    let il1 = make_signed_il(current_slot, committee[1], committee_root, vec![vec![3, 4]]);

    {
        let mut store = harness.chain.inclusion_list_store.lock();
        store.process_signed_inclusion_list(il0, true);
        store.process_signed_inclusion_list(il1, true);
    }

    // Query positions 0 and 1
    let result = harness
        .chain
        .get_inclusion_lists_by_committee_indices(current_slot, &[0, 1])
        .unwrap();
    assert_eq!(result.len(), 2);

    let result_validators: std::collections::HashSet<u64> =
        result.iter().map(|il| il.message.validator_index).collect();
    assert!(result_validators.contains(&committee[0]));
    assert!(result_validators.contains(&committee[1]));
}

#[tokio::test]
async fn get_inclusion_lists_filters_by_position() {
    let harness = heze_harness(1).await;
    let current_slot = head_slot(&harness);
    let (committee, committee_root) = committee_info(&harness, current_slot);

    // Insert IL for position 0 only
    let il0 = make_signed_il(current_slot, committee[0], committee_root, vec![]);
    {
        let mut store = harness.chain.inclusion_list_store.lock();
        store.process_signed_inclusion_list(il0, true);
    }

    // Query position 1 — should get nothing
    let result = harness
        .chain
        .get_inclusion_lists_by_committee_indices(current_slot, &[1])
        .unwrap();
    assert!(result.is_empty());

    // Query position 0 — should get the IL
    let result = harness
        .chain
        .get_inclusion_lists_by_committee_indices(current_slot, &[0])
        .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].message.validator_index, committee[0]);
}

#[tokio::test]
async fn get_inclusion_lists_out_of_range_indices_ignored() {
    let harness = heze_harness(1).await;
    let current_slot = head_slot(&harness);
    let (committee, committee_root) = committee_info(&harness, current_slot);

    let il0 = make_signed_il(current_slot, committee[0], committee_root, vec![]);
    {
        let mut store = harness.chain.inclusion_list_store.lock();
        store.process_signed_inclusion_list(il0, true);
    }

    // Index 999 is out of range — should be silently ignored
    let result = harness
        .chain
        .get_inclusion_lists_by_committee_indices(current_slot, &[0, 999])
        .unwrap();
    assert_eq!(result.len(), 1);
}

// ===========================================================================
// check_inclusion_list_satisfaction tests
// ===========================================================================

fn make_envelope(slot: Slot, transactions: Vec<Vec<u8>>) -> ExecutionPayloadEnvelope<E> {
    let tx_vecs: Vec<VariableList<u8, <E as EthSpec>::MaxBytesPerTransaction>> = transactions
        .into_iter()
        .map(|tx| VariableList::new(tx).unwrap())
        .collect();
    let tx_list = VariableList::new(tx_vecs).unwrap();

    let payload = ExecutionPayloadGloas {
        transactions: tx_list,
        ..Default::default()
    };

    ExecutionPayloadEnvelope {
        payload,
        execution_requests: ExecutionRequests::default(),
        builder_index: 0,
        beacon_block_root: Hash256::zero(),
        slot,
        state_root: Hash256::zero(),
    }
}

#[tokio::test]
async fn check_satisfaction_empty_il_store_returns_true() {
    let harness = heze_harness(2).await;
    let current_slot = head_slot(&harness);

    // No ILs stored — satisfaction check passes (empty IL txs)
    let envelope = make_envelope(current_slot, vec![]);
    assert!(harness.chain.check_inclusion_list_satisfaction(&envelope));
}

#[tokio::test]
async fn check_satisfaction_payload_includes_all_il_txs() {
    let harness = heze_harness(2).await;
    let current_slot = head_slot(&harness);
    let il_slot = Slot::new(current_slot.as_u64().saturating_sub(1));
    let (committee, committee_root) = committee_info(&harness, il_slot);

    let tx1 = vec![0xaa, 0xbb];
    let tx2 = vec![0xcc, 0xdd];

    // Insert IL at il_slot (N-1) with tx1 and tx2
    let il = make_signed_il(
        il_slot,
        committee[0],
        committee_root,
        vec![tx1.clone(), tx2.clone()],
    );
    {
        let mut store = harness.chain.inclusion_list_store.lock();
        store.process_signed_inclusion_list(il, true);
    }

    // Envelope at current_slot (N) includes both IL txs + extra
    let envelope = make_envelope(current_slot, vec![tx1, vec![0xee], tx2]);
    assert!(harness.chain.check_inclusion_list_satisfaction(&envelope));
}

#[tokio::test]
async fn check_satisfaction_missing_il_tx_returns_false() {
    let harness = heze_harness(2).await;
    let current_slot = head_slot(&harness);
    let il_slot = Slot::new(current_slot.as_u64().saturating_sub(1));
    let (committee, committee_root) = committee_info(&harness, il_slot);

    let required_tx = vec![0xaa, 0xbb];

    let il = make_signed_il(il_slot, committee[0], committee_root, vec![required_tx]);
    {
        let mut store = harness.chain.inclusion_list_store.lock();
        store.process_signed_inclusion_list(il, true);
    }

    // Envelope does NOT include the required transaction
    let envelope = make_envelope(current_slot, vec![vec![0xff]]);
    assert!(!harness.chain.check_inclusion_list_satisfaction(&envelope));
}

#[tokio::test]
async fn check_satisfaction_slot_zero_returns_true() {
    let harness = heze_harness(0).await;
    // Envelope at slot 0 — no previous slot, should return true
    let envelope = make_envelope(Slot::new(0), vec![]);
    assert!(harness.chain.check_inclusion_list_satisfaction(&envelope));
}

// ===========================================================================
// compute_inclusion_list_bits_for_slot tests
// ===========================================================================

#[tokio::test]
async fn compute_bits_empty_store_returns_all_zeros() {
    let harness = heze_harness(2).await;
    let current_slot = head_slot(&harness);

    let bits = harness
        .chain
        .compute_inclusion_list_bits_for_slot(current_slot);

    // No ILs stored — all bits should be 0
    assert!(bits.is_zero());
}

#[tokio::test]
async fn compute_bits_reflects_observed_validators() {
    let harness = heze_harness(2).await;
    let current_slot = head_slot(&harness);
    let il_slot = Slot::new(current_slot.as_u64().saturating_sub(1));
    let (committee, committee_root) = committee_info(&harness, il_slot);

    // Insert ILs for committee positions 0 and 2
    let il0 = make_signed_il(il_slot, committee[0], committee_root, vec![]);
    let il2 = make_signed_il(il_slot, committee[2], committee_root, vec![]);
    {
        let mut store = harness.chain.inclusion_list_store.lock();
        store.process_signed_inclusion_list(il0, true);
        store.process_signed_inclusion_list(il2, true);
    }

    let bits = harness
        .chain
        .compute_inclusion_list_bits_for_slot(current_slot);

    // Bits at positions 0 and 2 should be set
    assert!(bits.get(0).unwrap(), "bit 0 should be set");
    assert!(!bits.get(1).unwrap(), "bit 1 should NOT be set");
    assert!(bits.get(2).unwrap(), "bit 2 should be set");
}

#[tokio::test]
async fn compute_bits_slot_zero_returns_default() {
    let harness = heze_harness(0).await;

    // Slot 0 has no previous slot — should return all-zeros
    let bits = harness
        .chain
        .compute_inclusion_list_bits_for_slot(Slot::new(0));
    assert!(bits.is_zero());
}

#[tokio::test]
async fn compute_bits_equivocator_excluded() {
    let harness = heze_harness(2).await;
    let current_slot = head_slot(&harness);
    let il_slot = Slot::new(current_slot.as_u64().saturating_sub(1));
    let (committee, committee_root) = committee_info(&harness, il_slot);

    // Insert two different ILs from same validator (equivocation)
    let il_a = make_signed_il(il_slot, committee[0], committee_root, vec![]);
    let il_b = make_signed_il(il_slot, committee[0], committee_root, vec![vec![1]]);
    {
        let mut store = harness.chain.inclusion_list_store.lock();
        store.process_signed_inclusion_list(il_a, true);
        store.process_signed_inclusion_list(il_b, true);
    }

    let bits = harness
        .chain
        .compute_inclusion_list_bits_for_slot(current_slot);

    // Equivocator at position 0 should NOT have bit set
    assert!(!bits.get(0).unwrap(), "equivocator bit should NOT be set");
}

// ===========================================================================
// Block production: inclusion_list_bits embedded in Heze blocks
// ===========================================================================

#[tokio::test]
async fn block_production_embeds_inclusion_list_bits() {
    let harness = heze_harness(2).await;
    let head_state = harness.get_current_state();
    let current_slot = head_state.slot();
    let next_slot = current_slot + 1;

    // Insert an IL at current_slot (slot N) so the block at N+1 sees it
    let (committee, committee_root) = committee_info(&harness, current_slot);
    let il = make_signed_il(current_slot, committee[0], committee_root, vec![]);
    {
        let mut store = harness.chain.inclusion_list_store.lock();
        store.process_signed_inclusion_list(il, true);
    }

    harness.advance_slot();
    let ((signed_block, _blobs), _state) = harness.make_block(head_state, next_slot).await;

    // Extract the bid's inclusion_list_bits
    let block = signed_block.message();
    let body = block.body();
    let bid = body
        .signed_execution_payload_bid_heze()
        .expect("block should be Heze variant");

    let bits = &bid.message.inclusion_list_bits;

    // Position 0 should be set (we inserted an IL for committee[0])
    assert!(
        bits.get(0).unwrap(),
        "bit 0 should be set for observed IL at position 0"
    );
    // Position 1 should NOT be set (no IL inserted for that validator)
    assert!(
        !bits.get(1).unwrap(),
        "bit 1 should not be set — no IL for position 1"
    );
}

#[tokio::test]
async fn block_production_empty_il_store_all_bits_zero() {
    let harness = heze_harness(2).await;
    let head_state = harness.get_current_state();
    let next_slot = head_state.slot() + 1;

    harness.advance_slot();
    let ((signed_block, _blobs), _state) = harness.make_block(head_state, next_slot).await;

    let bid = signed_block
        .message()
        .body()
        .signed_execution_payload_bid_heze()
        .expect("block should be Heze variant");

    assert!(
        bid.message.inclusion_list_bits.is_zero(),
        "all bits should be zero when IL store is empty"
    );
}

// ===========================================================================
// get_best_execution_bid: Heze IL bits filtering
// ===========================================================================

/// When the external bid's inclusion_list_bits are a superset of the locally-observed
/// bits, get_best_execution_bid should return the bid.
#[tokio::test]
async fn heze_get_best_bid_inclusive_bits_accepted() {
    let harness = heze_harness(2).await;
    let head = harness.chain.canonical_head.cached_head();
    let state = &head.snapshot.beacon_state;
    let current_slot = state.slot();
    let target_slot = current_slot + 1;

    // Insert an IL at current_slot for committee[0]
    let (committee, committee_root) = committee_info(&harness, current_slot);
    let il = make_signed_il(current_slot, committee[0], committee_root, vec![]);
    {
        let mut store = harness.chain.inclusion_list_store.lock();
        store.process_signed_inclusion_list(il, true);
    }

    // Compute the actual local bits (a validator may appear at multiple committee positions
    // with a small validator set, so we need all positions covered)
    let local_bits = harness
        .chain
        .inclusion_list_store
        .lock()
        .get_inclusion_list_bits(&committee, committee_root, current_slot);

    // Build bid bits as a superset: set all local bits + one extra
    let mut bits = BitVector::default();
    for (i, &b) in local_bits.iter().enumerate() {
        if b {
            bits.set(i, true).unwrap();
        }
    }
    // Set an extra bit to verify superset is accepted
    let extra_pos = local_bits.iter().position(|b| !b).unwrap();
    bits.set(extra_pos, true).unwrap();

    let bid = SignedExecutionPayloadBidHeze::<E> {
        message: ExecutionPayloadBidHeze {
            slot: target_slot,
            builder_index: 0,
            value: 1000,
            parent_block_root: Hash256::zero(),
            inclusion_list_bits: bits,
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };
    harness.chain.execution_bid_pool.lock().insert(bid.into());

    let result = harness
        .chain
        .get_best_execution_bid(target_slot, Hash256::zero());
    assert!(
        result.is_some(),
        "bid with inclusive IL bits should be accepted"
    );
    assert_eq!(*result.unwrap().to_ref().message().value(), 1000);
}

/// When the external bid's inclusion_list_bits do NOT cover all locally-observed ILs,
/// get_best_execution_bid should reject it (return None).
#[tokio::test]
async fn heze_get_best_bid_non_inclusive_bits_rejected() {
    let harness = heze_harness(2).await;
    let head = harness.chain.canonical_head.cached_head();
    let state = &head.snapshot.beacon_state;
    let current_slot = state.slot();
    let target_slot = current_slot + 1;

    // Find two DISTINCT committee members (skip duplicates in small validator sets)
    let (committee, committee_root) = committee_info(&harness, current_slot);
    let vi0 = committee[0];
    let (_, &vi1) = committee
        .iter()
        .enumerate()
        .find(|&(_, &vi)| vi != vi0)
        .expect("need at least 2 distinct validators in committee");

    // Insert ILs for both distinct validators
    let il0 = make_signed_il(current_slot, vi0, committee_root, vec![]);
    let il1 = make_signed_il(current_slot, vi1, committee_root, vec![]);
    {
        let mut store = harness.chain.inclusion_list_store.lock();
        store.process_signed_inclusion_list(il0, true);
        store.process_signed_inclusion_list(il1, true);
    }

    // Build bits that only cover vi0's positions but NOT vi1's
    let mut bits = BitVector::default();
    for (i, &vi) in committee.iter().enumerate() {
        if vi == vi0 {
            bits.set(i, true).unwrap();
        }
    }
    // vi1 positions are NOT set — bid is missing a locally-observed IL

    let bid = SignedExecutionPayloadBidHeze::<E> {
        message: ExecutionPayloadBidHeze {
            slot: target_slot,
            builder_index: 0,
            value: 5000,
            parent_block_root: Hash256::zero(),
            inclusion_list_bits: bits,
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };
    harness.chain.execution_bid_pool.lock().insert(bid.into());

    let result = harness
        .chain
        .get_best_execution_bid(target_slot, Hash256::zero());
    assert!(
        result.is_none(),
        "bid with non-inclusive IL bits should be rejected"
    );
}

/// When the IL store is empty (no ILs observed), any bid's inclusion_list_bits
/// are trivially inclusive, so the bid should be accepted.
#[tokio::test]
async fn heze_get_best_bid_empty_store_accepts_any_bits() {
    let harness = heze_harness(2).await;
    let head = harness.chain.canonical_head.cached_head();
    let state = &head.snapshot.beacon_state;
    let target_slot = state.slot() + 1;

    // No ILs inserted — store is empty

    let bid = SignedExecutionPayloadBidHeze::<E> {
        message: ExecutionPayloadBidHeze {
            slot: target_slot,
            builder_index: 0,
            value: 999,
            parent_block_root: Hash256::zero(),
            inclusion_list_bits: BitVector::default(), // all-zero bits
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };
    harness.chain.execution_bid_pool.lock().insert(bid.into());

    let result = harness
        .chain
        .get_best_execution_bid(target_slot, Hash256::zero());
    assert!(
        result.is_some(),
        "bid should be accepted when no local ILs observed"
    );
}

/// When the highest-value Heze bid has non-inclusive bits,
/// get_best_execution_bid should return None.
#[tokio::test]
async fn heze_get_best_bid_highest_value_rejected_returns_none() {
    let harness = heze_harness(2).await;
    let head = harness.chain.canonical_head.cached_head();
    let state = &head.snapshot.beacon_state;
    let current_slot = state.slot();
    let target_slot = current_slot + 1;

    // Insert an IL for committee[0]
    let (committee, committee_root) = committee_info(&harness, current_slot);
    let il = make_signed_il(current_slot, committee[0], committee_root, vec![]);
    {
        let mut store = harness.chain.inclusion_list_store.lock();
        store.process_signed_inclusion_list(il, true);
    }

    // High-value bid with all-zero bits — non-inclusive since local has at least bit 0
    let bid_high = SignedExecutionPayloadBidHeze::<E> {
        message: ExecutionPayloadBidHeze {
            slot: target_slot,
            builder_index: 0,
            value: 10000,
            parent_block_root: Hash256::zero(),
            inclusion_list_bits: BitVector::default(), // all-zero — non-inclusive
            ..Default::default()
        },
        signature: bls::Signature::empty(),
    };

    harness
        .chain
        .execution_bid_pool
        .lock()
        .insert(bid_high.into());

    let result = harness
        .chain
        .get_best_execution_bid(target_slot, Hash256::zero());
    assert!(
        result.is_none(),
        "best bid with non-inclusive bits should be rejected"
    );
}

// ===========================================================================
// Pre-Heze fork early-return tests
// ===========================================================================

/// Build a Gloas (pre-Heze) harness to test pre-fork early returns.
async fn gloas_harness(num_blocks: usize) -> BeaconChainHarness<EphemeralHarnessType<E>> {
    let mut spec = ForkName::Gloas.make_genesis_spec(E::default_spec());
    spec.target_aggregators_per_committee = 1 << 32;

    let harness = BeaconChainHarness::builder(MainnetEthSpec)
        .spec(Arc::new(spec))
        .keypairs(KEYPAIRS[0..VALIDATOR_COUNT].to_vec())
        .fresh_ephemeral_store()
        .mock_execution_layer()
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

/// Pre-Heze: check_inclusion_list_satisfaction always returns true.
#[tokio::test]
async fn check_satisfaction_pre_heze_returns_true() {
    let harness = gloas_harness(2).await;
    let current_slot = head_slot(&harness);

    // Even with a fabricated envelope, pre-Heze should always return true
    let envelope = make_envelope(current_slot, vec![]);
    assert!(
        harness.chain.check_inclusion_list_satisfaction(&envelope),
        "pre-Heze should always return true"
    );
}

/// Pre-Heze: compute_inclusion_list_bits_for_slot returns all-zeros.
#[tokio::test]
async fn compute_bits_pre_heze_returns_default() {
    let harness = gloas_harness(2).await;
    let current_slot = head_slot(&harness);

    let bits = harness
        .chain
        .compute_inclusion_list_bits_for_slot(current_slot);
    assert!(bits.is_zero(), "pre-Heze should return all-zero bits");
}

/// Pre-Heze: verify_inclusion_list_for_gossip rejects with PreHezeFork error.
#[tokio::test]
async fn gossip_il_reject_pre_heze_fork() {
    let harness = gloas_harness(1).await;
    let current_slot = harness.chain.slot().unwrap();

    // Create a dummy IL — the fork check should fire before any other validation
    let signed_il = make_signed_il(current_slot, 0, Hash256::zero(), vec![]);

    let err = unwrap_il_err(
        harness.chain.verify_inclusion_list_for_gossip(signed_il),
        "should reject pre-Heze IL",
    );
    assert!(
        matches!(err, InclusionListError::PreHezeFork { .. }),
        "expected PreHezeFork, got {err:?}"
    );
}
