# Upstream Gloas Code Review: Payload Bid Processing

> Analysis of PR #8801 (b8072c5b7) - execution payload bid consensus

## Overview

Upstream lighthouse has implemented core ePBS payload bid processing. The implementation is **clean, well-tested, and passing EF tests**. This is a strong candidate for cherry-picking.

## Key Implementation Details

### New Function: `process_execution_payload_bid()`

Located in: `consensus/state_processing/src/per_block_processing.rs`

**What it does**:
1. Validates the execution payload bid signature
2. Checks builder registration and balance
3. Verifies bid parameters (slot, parent, randao, blob commitments)
4. Records pending payment in state
5. Caches the bid in `state.latest_execution_payload_bid`

### Validation Logic

**Self-builds** (builder_index == BUILDER_INDEX_SELF_BUILD):
- Amount MUST be zero
- Signature MUST be infinity point (no sig verification needed)

**External builders**:
- Builder must be active at finalized epoch: `deposit_epoch < finalized_epoch && withdrawable_epoch == far_future`
- Builder must have sufficient balance (including pending withdrawals/payments)
- Signature verification against builder's registered pubkey
- Domain: `Domain::BeaconBuilder`

### Bid Validation Checks

All verified with `block_verify!` macro (returns `BlockProcessingError` on failure):

1. **Slot match**: `bid.slot == block.slot()`
2. **Parent block hash**: `bid.parent_block_hash == state.latest_block_hash()`
3. **Parent block root**: `bid.parent_block_root == block.parent_root()`
4. **Prev randao**: `bid.prev_randao == state.get_randao_mix(current_epoch)`
5. **Blob commitment limit**: `bid.blob_kzg_commitments.len() <= max_blobs_per_block`

### Pending Payment Recording

For bids with `amount > 0`:
```rust
let pending_payment = BuilderPendingPayment {
    weight: 0,  // Weight accumulated during epoch processing via PTC attestations
    withdrawal: BuilderPendingWithdrawal {
        fee_recipient: bid.fee_recipient,
        amount,
        builder_index,
    },
};
```

Payment index calculation:
```rust
payment_index = SLOTS_PER_EPOCH + (bid.slot % SLOTS_PER_EPOCH)
```

This means:
- Slots 0-31 → payment indices 32-63
- Payments deferred to next epoch boundary
- Index offset prevents conflict with per-epoch data

### State Changes

New beacon state fields (accessed via getters):
- `latest_execution_payload_bid()` - caches the current bid
- `builder_pending_payments()` - array of pending payments (length = 2 * SLOTS_PER_EPOCH)
- `builder_pending_withdrawals()` - queue of withdrawals to process

New helper method:
```rust
state.get_pending_balance_to_withdraw_for_builder(builder_index) -> u64
```
Sums pending withdrawals + pending payments for a builder (used in balance check).

### New Error Types

`ExecutionPayloadBidInvalid` enum with detailed variants:
- `SelfBuildNonZeroAmount`
- `BadSignature`
- `BuilderNotActive(u64)`
- `InsufficientBalance { builder_index, builder_balance, bid_value }`
- `SlotMismatch { bid_slot, block_slot }`
- `ParentBlockHashMismatch`
- `ParentBlockRootMismatch`
- `PrevRandaoMismatch`
- `ExcessBlobCommitments { max, bid }`

All errors are descriptive and include relevant context for debugging.

### Integration into Block Processing

In `per_block_processing()`, the call happens AFTER withdrawals, BEFORE execution payload:

```rust
if state.fork_name_unchecked().gloas_enabled() {
    withdrawals::gloas::process_withdrawals::<E>(state, spec)?;
    // TODO(EIP-7732): process execution payload bid
    // This TODO is now implemented!
}
```

### Test Coverage

New EF test handler: `ExecutionPayloadBidBlock<E>`
- Tests decode `block.ssz_snappy` (full beacon block with embedded bid)
- Calls `process_execution_payload_bid()` with `VerifySignatures::True`
- Enabled for gloas fork only
- Tests cover: valid bids, self-builds, invalid signatures, balance checks, parameter mismatches

Test files enabled:
- ~~`tests/.*/gloas/operations/block_header/.*`~~ (no longer ignored)
- ~~`tests/.*/gloas/operations/execution_payload_bid/.*`~~ (no longer ignored)

**All tests passing** according to PR description.

## Signature Verification

New signature set function: `execution_payload_bid_signature_set()`

Located in: `consensus/state_processing/src/per_block_processing/signature_sets.rs`

**Logic**:
- Returns `Ok(None)` for self-builds (infinity signature, no verification)
- Returns `Ok(Some(SignatureSet))` for external builders
- Domain: `Domain::BeaconBuilder`
- Signing root: `execution_payload_bid.signing_root(domain)`
- Pubkey retrieval via `get_builder_pubkey_from_state(state, builder_index)`

## Builder Type Extensions

New method on `Builder` struct:

```rust
impl Builder {
    pub fn is_active_at_finalized_epoch(&self, finalized_epoch: Epoch, spec: &ChainSpec) -> bool {
        self.deposit_epoch < finalized_epoch && self.withdrawable_epoch == spec.far_future_epoch
    }
}
```

This implements the spec's `is_active_builder` predicate.

## Code Quality Assessment

**Strengths**:
1. ✅ Clear error messages with context
2. ✅ Comprehensive validation (9 different check types)
3. ✅ Separation of self-build vs external builder logic
4. ✅ Well-integrated into existing block processing flow
5. ✅ Test coverage via EF tests
6. ✅ Follows existing lighthouse patterns (block_verify! macro, signature_sets)
7. ✅ Descriptive variable names
8. ✅ Proper use of safe_arith for overflow protection

**Potential concerns**:
- Weight field in `BuilderPendingPayment` initialized to 0, accumulated later (need to verify epoch processing)
- Payment index calculation assumes specific state layout (need to verify state spec)
- Dependency on `get_builder_pubkey_from_state` (need to verify builder registry implementation)

## Comparison with Spec

Need to verify against:
- [consensus-specs/specs/gloas/beacon-chain.md](https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md)
  - `process_execution_payload_bid()` function
  - `is_active_builder()` predicate
  - Builder balance calculation
  - Payment recording logic

## Cherry-Pick Recommendation

**RECOMMEND CHERRY-PICK** with conditions:

1. ✅ Code quality is high
2. ✅ Tests are passing
3. ✅ Integration is clean
4. ⚠️ Need to verify dependencies are met:
   - `Builder` type exists in our fork ❌ **NOT YET**
   - `SignedExecutionPayloadBid` type exists ❌ **NOT YET**
   - State accessors (`builder_pending_payments`, etc.) exist ❌ **NOT YET**
   - `BUILDER_INDEX_SELF_BUILD` constant defined ❌ **NOT YET**

**CRITICAL DISCOVERY**: Found prerequisite types PR!
- **a39e99155** - Gloas(EIP-7732): Containers / Constants (#7923) - Dec 16, 2025
- 52 files changed, +930/-689 lines
- This PR introduces ALL the gloas types needed for bid processing
- Must be cherry-picked FIRST before any other gloas code

**Strategy**:
- Cherry-pick order:
  1. **a39e99155** - Types and constants (Dec 16, 2025) ← START HERE
  2. **21cabba1a** - Updated consensus types for Gloas 1.7.0-alpha.1 (#8688)
  3. **b8072c5b7** - Gloas payload bid consensus (#8801) ← Bid processing
  4. **26db01642** - Gloas epoch processing (#8808) ← Epoch processing
  5. **68ad9758a** - Gloas attestation verification (#8705) ← Attestation verification

- Alternative: cherry-pick in bulk as a branch merge
- Risk: 52-file types PR may have conflicts with our v8.0.1 base

**Decision point**: Try cherry-picking types PR first, or study it and implement types from spec?

## Next Steps

1. Check if upstream has types PR that precedes this one
2. Review `consensus/types` changes for gloas types
3. Document the full gloas type hierarchy needed
4. Then make cherry-pick decision

## Notes

- Upstream is moving fast on gloas (3 major PRs in 24h)
- Implementation quality is production-ready
- Cherry-picking looks viable but requires type foundation first
- Spec compliance verification still needed (external consensus-specs comparison)
