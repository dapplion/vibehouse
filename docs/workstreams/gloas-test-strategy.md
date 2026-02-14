# Gloas Test Strategy

## Overview

Phase 1 and 2 (Types & State Transitions) are code-complete. This document defines the testing strategy to validate the implementation.

## Test Categories

### 1. Consensus Spec Tests (Priority 1)

**Location**: `testing/ef_tests/consensus-spec-tests/tests/{mainnet,minimal}/gloas/operations/`

**Available test vectors**:
- `payload_attestation/` - 9 test cases:
  - `process_payload_attestation_payload_present` ✅
  - `process_payload_attestation_payload_not_present` ✅
  - `process_payload_attestation_partial_participation` ✅
  - `process_payload_attestation_invalid_signature` ✅
  - `process_payload_attestation_wrong_slot` ✅ (expected failure)
  - `process_payload_attestation_too_old_slot` ✅ (expected failure)
  - `process_payload_attestation_future_slot` ✅ (expected failure)
  - `process_payload_attestation_no_attesting_indices` ✅ (expected failure)
  - `process_payload_attestation_invalid_beacon_block_root` ✅ (expected failure)
  - `process_payload_attestation_cross_epoch_wrong_domain` ✅ (expected failure)

**Status**: Test handlers added to `testing/ef_tests/tests/tests.rs`, but need to verify with actual test run.

**Handlers implemented**:
```rust
OperationsHandler::<MinimalEthSpec, _>::new(
    "operations_execution_payload_bid",
    "consensus-spec-tests",
    "tests",
)

OperationsHandler::<MinimalEthSpec, _>::new(
    "operations_payload_attestation",
    "consensus-spec-tests",
    "tests",
)
```

**Expected behavior**:
- Valid cases should pass and update state correctly
- Invalid cases should return specific BlockProcessingError variants

### 2. Unit Tests (Priority 2)

**Location**: `consensus/state_processing/src/per_block_processing/gloas.rs` (at end of file)

**Test skeletons exist** (12 total):

#### ExecutionPayloadBid Tests
1. `test_process_execution_payload_bid_self_build`
   - Verify self-build bid accepted with value=0 and empty signature
   - Should not create pending payment

2. `test_process_execution_payload_bid_external_builder`
   - Valid external builder bid
   - Should create pending payment in correct slot index
   - Should verify signature correctly

3. `test_process_execution_payload_bid_insufficient_balance`
   - Builder balance < bid value
   - Should return `PayloadBidInvalid` error

4. `test_process_execution_payload_bid_inactive_builder`
   - Builder not active (deposited after finalized or withdrawable)
   - Should return `PayloadBidInvalid` error

5. `test_process_execution_payload_bid_wrong_slot`
   - Bid slot != state slot
   - Should return `PayloadBidInvalid` error

#### PayloadAttestation Tests
6. `test_process_payload_attestation_quorum_reached`
   - PTC attestation with ≥307 attesters (60% of 512)
   - Should set execution_payload_availability bit
   - Should trigger builder payment if payload_present=true

7. `test_process_payload_attestation_quorum_not_reached`
   - PTC attestation with <307 attesters
   - Should accept but NOT trigger payment or set availability

8. `test_process_payload_attestation_wrong_slot`
   - Attestation slot != state slot
   - Should return `PayloadAttestationInvalid::WrongSlot`

#### PTC Committee Tests
9. `test_get_ptc_committee_deterministic`
   - Same slot + state → same PTC committee
   - Run twice, compare results

10. `test_get_ptc_committee_size`
    - With ≥512 validators, PTC should have exactly 512 members
    - All members should be valid indices

#### Indexed Attestation Tests
11. `test_get_indexed_payload_attestation`
    - Convert PayloadAttestation → IndexedPayloadAttestation
    - Verify attesting_indices correctly extracted from aggregation_bits

12. `test_indexed_payload_attestation_sorted`
    - Indices must be sorted in ascending order
    - Test with unsorted input → should error

### 3. Integration Tests (Priority 3)

**Location**: `beacon_node/beacon_chain/tests/` (new file: `gloas_integration.rs`)

**Test scenarios**:

#### Full Block Processing
- Create a gloas BeaconBlock with:
  - ExecutionPayloadBid in proposer block
  - PayloadAttestations from PTC
- Process through full `per_block_processing()`
- Verify state updates correctly

#### Fork Transition
- Fulu state at epoch boundary
- Process first gloas block
- Verify:
  - New gloas state fields initialized
  - Old fields preserved
  - Fork choice updated

#### Builder Payment Flow
1. Slot N: Proposer chooses builder bid, creates pending payment
2. Slot N+1: PTC attests, reaches quorum
3. Verify:
   - Builder balance decreased
   - Proposer balance increased (when TODO resolved)
   - Payment marked as processed

### 4. Property-Based Tests (Priority 4)

**Tool**: `proptest` or `quickcheck`

**Properties to test**:

1. **PTC Committee Uniqueness**: No duplicate validator indices in PTC
2. **PTC Committee Determinism**: Same inputs → same outputs
3. **Payment Idempotency**: Multiple attestations for same slot don't double-pay
4. **Balance Conservation**: Total balance unchanged (builder pays, proposer receives)
5. **Signature Verification**: Invalid sigs always rejected, valid sigs always accepted

## Test Utilities Needed

### State Builders
```rust
// consensus/state_processing/src/per_block_processing/tests/test_utils.rs

/// Create a minimal Gloas state with specified validator count
pub fn new_gloas_state<E: EthSpec>(
    validator_count: usize,
    builder_count: usize,
    spec: &ChainSpec,
) -> BeaconState<E>

/// Add a builder to state with specified balance
pub fn add_builder<E: EthSpec>(
    state: &mut BeaconState<E>,
    pubkey: PublicKeyBytes,
    balance: u64,
    execution_address: Address,
) -> BuilderIndex

/// Create validators and activate them
pub fn populate_validators<E: EthSpec>(
    state: &mut BeaconState<E>,
    count: usize,
    balance: u64,
)
```

### Message Builders
```rust
/// Create a valid ExecutionPayloadBid for testing
pub fn make_execution_payload_bid<E: EthSpec>(
    slot: Slot,
    builder_index: BuilderIndex,
    value: u64,
    parent_block_root: Hash256,
) -> ExecutionPayloadBid<E>

/// Sign an ExecutionPayloadBid
pub fn sign_bid<E: EthSpec>(
    bid: &ExecutionPayloadBid<E>,
    secret_key: &SecretKey,
    fork: &Fork,
    genesis_validators_root: Hash256,
    spec: &ChainSpec,
) -> SignedExecutionPayloadBid<E>

/// Create a PayloadAttestation with specified attesters
pub fn make_payload_attestation<E: EthSpec>(
    slot: Slot,
    beacon_block_root: Hash256,
    payload_present: bool,
    attesting_indices: &[u64],
    ptc_committee: &[u64],
) -> PayloadAttestation<E>

/// Sign a PayloadAttestation
pub fn sign_payload_attestation<E: EthSpec>(
    attestation: &PayloadAttestation<E>,
    secret_keys: &[SecretKey],
    fork: &Fork,
    genesis_validators_root: Hash256,
    spec: &ChainSpec,
) -> PayloadAttestation<E>
```

## Test Execution Plan

### Phase 1: Spec Tests (immediate, when cargo available)
1. Run minimal spec tests: `cargo nextest run --release --test tests --features ef_tests minimal gloas`
2. Expected: Some failures initially (fix incrementally)
3. Target: 100% pass rate for all gloas operation tests

### Phase 2: Unit Tests (week 1)
1. Implement all 12 unit test skeletons
2. Create test utilities as needed
3. Run: `cargo nextest run --release --lib state_processing gloas`
4. Target: 100% pass rate, no `#[ignore]`

### Phase 3: Integration Tests (week 2)
1. Write full block processing integration tests
2. Test fork transition
3. Test builder payment flow end-to-end

### Phase 4: Coverage & Property Tests (ongoing)
1. Measure coverage: `cargo llvm-cov --lib state_processing --html`
2. Target: >85% coverage for gloas.rs
3. Add property tests for invariants
4. Fuzz test with `cargo-fuzz` if time permits

## Known Issues & TODOs

### In Implementation
- **Proposer balance increase**: Need proposer_index from ConsensusContext
  - Payment flow processes builder deduction but not proposer credit
  - Low priority: payment is recorded, just not applied yet

### In Tests
- **Test state builder**: Need proper Gloas state constructor
  - Current `get_gloas_state()` in gloas.rs has `todo!()`
  - Must implement clean upgrade path or mock state

## Success Criteria

✅ **Phase 1 & 2 Complete When**:
- All spec tests pass (9 payload_attestation tests)
- All unit tests pass (12 tests)
- No `#[ignore]` on any gloas test
- `make test-ef` succeeds with gloas tests included

✅ **Ready for Phase 3 (Fork Choice) When**:
- Above criteria met
- Integration tests written and passing
- Coverage >80% on gloas.rs

## Resources

- **Spec**: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md
- **Upstream**: https://github.com/sigp/lighthouse/pull/8806
- **Test vectors**: `testing/ef_tests/consensus-spec-tests/tests/mainnet/gloas/`
- **Test runner**: `testing/ef_tests/tests/tests.rs`

---

**Status**: Strategy documented. Awaiting Rust toolchain to begin execution. 🎵
