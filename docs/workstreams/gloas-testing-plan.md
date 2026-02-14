# Gloas Testing Plan

## Status: Blocked on Rust toolchain

Tests have been scaffolded in `consensus/state_processing/src/per_block_processing/gloas.rs` but cannot be implemented without a working Rust environment (cargo/rustc).

## Test Coverage Needed

### Phase 2: State Transition Tests (12 tests)

#### process_execution_payload_bid tests (5 tests)

1. **test_process_execution_payload_bid_self_build** ✅ Scaffolded
   - Test that self-build bids are accepted with:
     - `builder_index == BUILDER_INDEX_SELF_BUILD` (u64::MAX)
     - `value == 0`
     - `signature.is_empty() == true`
   - Expected: Success

2. **test_process_execution_payload_bid_external_builder** ✅ Scaffolded
   - Test that external builder bids are validated correctly:
     - Builder exists in state.builders
     - Builder is active (finalized)
     - Builder has sufficient balance
     - Signature verifies (with VerifySignatures::True)
   - Expected: Success

3. **test_process_execution_payload_bid_insufficient_balance** ✅ Scaffolded
   - Setup: Builder with balance < bid value
   - Expected: `PayloadBidInvalid { reason: "builder balance ... insufficient for bid value ..." }`

4. **test_process_execution_payload_bid_inactive_builder** ✅ Scaffolded
   - Setup: Builder that's not finalized or has been withdrawn
   - Expected: `PayloadBidInvalid { reason: "builder ... is not active" }`

5. **test_process_execution_payload_bid_wrong_slot** ✅ Scaffolded
   - Setup: Bid slot != state.slot()
   - Expected: `PayloadBidInvalid { reason: "bid slot ... does not match state slot ..." }`

#### process_payload_attestation tests (3 tests)

6. **test_process_payload_attestation_quorum_reached** ✅ Scaffolded
   - Setup: Attestation with >= 307 attesters (60% of 512)
   - Expected:
     - `execution_payload_availability` bit set for slot
     - Builder payment processed (if payload_present)
     - Builder balance decreased
     - Proposer balance increased (TODO in implementation)

7. **test_process_payload_attestation_quorum_not_reached** ✅ Scaffolded
   - Setup: Attestation with < 307 attesters
   - Expected:
     - Attestation accepted
     - No payment processing
     - No state changes beyond acceptance

8. **test_process_payload_attestation_wrong_slot** ✅ Scaffolded
   - Setup: Attestation slot != state.slot()
   - Expected: `PayloadAttestationInvalid::WrongSlot { expected, actual }`

#### get_ptc_committee tests (2 tests)

9. **test_get_ptc_committee_deterministic** ✅ Scaffolded
   - Setup: Same state, same slot, call twice
   - Expected: Identical PTC committee both times

10. **test_get_ptc_committee_size** ✅ Scaffolded
    - Setup: State with >= 512 active validators
    - Expected: Committee size == 512

#### get_indexed_payload_attestation tests (2 tests)

11. **test_get_indexed_payload_attestation** ✅ Scaffolded
    - Setup: PayloadAttestation with aggregation bits set
    - Expected:
      - IndexedPayloadAttestation returned
      - attesting_indices matches aggregation bits
      - data and signature copied correctly

12. **test_indexed_payload_attestation_sorted** ✅ Scaffolded
    - Setup: PayloadAttestation with out-of-order aggregation
    - Expected:
      - attesting_indices are sorted
      - or IndicesNotSorted error if shuffle produces unsorted

## Test Utilities Needed

### State Builder for Gloas

Need a helper function to create valid Gloas test states:

```rust
fn get_gloas_state<E: EthSpec>(
    validator_count: usize,
    builder_count: usize,
    spec: &ChainSpec,
) -> BeaconState<E>
```

Requirements:
- Creates validators with random pubkeys
- Initializes Gloas-specific fields:
  - `builders` list
  - `builder_pending_payments`
  - `builder_pending_withdrawals`
  - `execution_payload_availability`
  - `latest_execution_payload_bid`
- Sets finalized_checkpoint
- Activates validators at epoch 0

### Builder Helper

```rust
fn create_test_builder(
    index: u64,
    balance: u64,
    activation_epoch: Epoch,
    exit_epoch: Epoch,
) -> Builder
```

### Bid Helper

```rust
fn create_test_bid<E: EthSpec>(
    slot: Slot,
    builder_index: u64,
    value: u64,
    spec: &ChainSpec,
) -> ExecutionPayloadBid<E>

fn sign_bid<E: EthSpec>(
    bid: &ExecutionPayloadBid<E>,
    secret_key: &SecretKey,
    state: &BeaconState<E>,
    spec: &ChainSpec,
) -> SignedExecutionPayloadBid<E>
```

### Attestation Helper

```rust
fn create_test_payload_attestation<E: EthSpec>(
    slot: Slot,
    beacon_block_root: Hash256,
    payload_present: bool,
    num_attesters: usize,
    ptc_size: usize,
) -> PayloadAttestation<E>
```

## Integration Test Plan

Once unit tests pass, integration tests needed:

1. **Full ePBS flow**:
   - Proposer creates block
   - Builder submits bid
   - Bid is processed
   - Payload is revealed
   - PTC attests
   - Payment is made

2. **Builder withholding**:
   - Builder submits bid
   - Builder doesn't reveal payload
   - PTC attests payload_present=false
   - No payment made

3. **Partial PTC participation**:
   - Some PTC members attest
   - Test quorum thresholds (59%, 60%, 61%)

4. **Multiple bids per slot**:
   - Multiple builders bid
   - Only highest bid is accepted (proposer choice)

## Spec Test Coverage

Need to ensure spec test runner handles gloas operations:

- `testing/ef_tests/tests/tests.rs` - add handlers for:
  - `operations/execution_payload_bid`
  - `operations/payload_attestation`

Existing spec test vectors found in:
```
./testing/ef_tests/consensus-spec-tests/tests/mainnet/gloas/operations/execution_payload_bid/
./testing/ef_tests/consensus-spec-tests/tests/mainnet/gloas/operations/payload_attestation/
```

## Next Steps (when toolchain available)

1. Implement `get_gloas_state` test helper
2. Implement all 12 unit tests
3. Run tests: `cargo test gloas`
4. Fix any failures
5. Add spec test handlers
6. Run spec tests: `make test-ef`
7. Fix spec test failures
8. Commit passing tests

## Blocked By

- Rust toolchain not available on this machine
- Cannot run `cargo test` or `cargo check`
- All tests must be written and verified later when toolchain is available
