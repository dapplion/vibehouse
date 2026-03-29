#![cfg(not(debug_assertions))]

//! Integration tests for Heze FOCIL (EIP-7805) beacon chain functions:
//!
//! - `get_inclusion_lists_by_committee_indices` — query cached ILs by committee position
//! - `check_inclusion_list_satisfaction` — envelope IL satisfaction check
//! - `compute_inclusion_list_bits_for_slot` — bit computation for block production

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
