# Gloas Types Implementation Plan

> cherry-pick infeasible (35 conflicts), implementing from scratch using upstream as reference

## Cherry-Pick Attempt Failed

Attempted: `git cherry-pick a39e99155` (Gloas types PR from Dec 2025)
Result: **35 conflicting files**

Conflicts include:
- Makefile, beacon_chain, execution_layer code
- Major type refactors (attestation/mod.rs deleted in HEAD, exists in upstream)
- Builder types directory structure mismatch
- Execution types directory reorganization

**Root cause**: Our v8.0.1 fork base (Dec 3, 2025) vs types PR (Dec 16, 2025) + 2 months of upstream changes = massive divergence.

**Decision**: Implement types fresh, using upstream PR as reference + consensus-specs as source of truth.

## New Types to Implement

### Payload Attestation Types (New to Gloas)

1. **`PayloadAttestationData`** - `consensus/types/src/attestation/payload_attestation_data.rs`
   - Contains: slot, beacon_block_root, hash of execution payload
   - Attests to payload delivery for PTC

2. **`PayloadAttestationMessage`** - `consensus/types/src/attestation/payload_attestation_message.rs`
   - Contains: validator_index, payload_attestation_data

3. **`PayloadAttestation`** - `consensus/types/src/attestation/payload_attestation.rs`
   - Contains: PayloadAttestationMessage + aggregation_bits + signature

4. **`IndexedPayloadAttestation`** - `consensus/types/src/attestation/indexed_payload_attestation.rs`
   - Contains: attesting_indices (list of validator indices), PayloadAttestationData, signature
   - Used for slashing detection

### Builder Registry Types (New to Gloas)

5. **`Builder`** - `consensus/types/src/builder/builder.rs`
   - Fields:
     - pubkey: PublicKeyBytes
     - balance: u64
     - withdrawal_address: Address
     - deposit_epoch: Epoch
     - withdrawable_epoch: Epoch
   - Methods:
     - `is_active_at_finalized_epoch(finalized_epoch, spec) -> bool`

6. **`BuilderIndex`** - type alias for `u64`

7. **`BuilderPendingPayment`** - `consensus/types/src/builder/builder_pending_payment.rs`
   - Fields:
     - weight: u64 (accumulated from PTC attestations)
     - withdrawal: BuilderPendingWithdrawal

8. **`BuilderPendingWithdrawal`** - `consensus/types/src/builder/builder_pending_withdrawal.rs`
   - Fields:
     - fee_recipient: Address
     - amount: u64
     - builder_index: BuilderIndex

### Execution Payload Bid Types (New to Gloas)

9. **`ExecutionPayloadBid`** - `consensus/types/src/execution/execution_payload_bid.rs`
   - Fields:
     - slot: Slot
     - builder_index: BuilderIndex
     - parent_block_root: Hash256
     - parent_block_hash: ExecutionBlockHash
     - value: u64
     - blob_kzg_commitments: List<KzgCommitment, MAX_BLOBS_PER_BLOCK>
     - prev_randao: Hash256
     - fee_recipient: Address
   - Methods:
     - `signing_root(domain) -> Hash256`

10. **`SignedExecutionPayloadBid`** - `consensus/types/src/execution/signed_execution_payload_bid.rs`
    - Fields:
      - message: ExecutionPayloadBid
      - signature: Signature

### Execution Payload Envelope Types (Gloas changes)

11. **`ExecutionPayloadEnvelope`** - `consensus/types/src/execution/execution_payload_envelope.rs`
    - Contains: ExecutionPayload + builder_index + blobs + proofs
    - Revealed by builder in phase 2

12. **`SignedExecutionPayloadEnvelope`** - `consensus/types/src/execution/signed_execution_payload_envelope.rs`
    - Contains: ExecutionPayloadEnvelope + signature

### BeaconBlockBody Changes (Gloas)

13. **Update `BeaconBlockBody<Gloas>`**:
    - Replace `execution_payload: Option<ExecutionPayload>` with:
      - `signed_execution_payload_bid: SignedExecutionPayloadBid`
    - Add: `payload_attestations: List<PayloadAttestation, MAX_PAYLOAD_ATTESTATIONS_ELECTRA>`

### BeaconState Changes (Gloas)

14. **Update `BeaconState<Gloas>`**:
    - Add: `builders: List<Builder, MAX_BUILDERS>`
    - Add: `builder_pending_payments: Vector<BuilderPendingPayment, BUILDER_PENDING_PAYMENTS_LENGTH>`
    - Add: `builder_pending_withdrawals: List<BuilderPendingWithdrawal, MAX_BUILDER_PENDING_WITHDRAWALS>`
    - Add: `latest_execution_payload_bid: ExecutionPayloadBid`
    - Add: `execution_payload_availability: Bitlist<MAX_PTC_SIZE>`

### Constants (Gloas)

15. **New constants in `ChainSpec`**:
    - `BUILDER_INDEX_SELF_BUILD: u64 = 2^64 - 1`
    - `MAX_BUILDERS: usize`
    - `BUILDER_PENDING_PAYMENTS_LENGTH: usize = 2 * SLOTS_PER_EPOCH`
    - `MAX_BUILDER_PENDING_WITHDRAWALS: usize`
    - `MAX_PAYLOAD_ATTESTATIONS_ELECTRA: usize`
    - `MAX_PTC_SIZE: usize = 512` (Payload Timeliness Committee)
    - `PTC_SIZE: usize = 512`
    - `PTC_ACTIVATION_NUMERATOR: u64`
    - `PTC_ACTIVATION_DENOMINATOR: u64`

### Domain Types

16. **New domain**: `Domain::BeaconBuilder`
    - Used for builder bid signature verification

## Implementation Order

Implement in dependency order to avoid forward references:

### Phase 1: Basic Types (No dependencies)
1. `BuilderIndex` (type alias)
2. `Builder` struct + methods
3. `BuilderPendingWithdrawal`
4. `BuilderPendingPayment`
5. Constants in ChainSpec

### Phase 2: Execution Types
6. `ExecutionPayloadBid` + SSZ derives
7. `SignedExecutionPayloadBid`
8. `ExecutionPayloadEnvelope` (if not exists)
9. `SignedExecutionPayloadEnvelope`

### Phase 3: Attestation Types
10. `PayloadAttestationData`
11. `PayloadAttestationMessage`
12. `PayloadAttestation`
13. `IndexedPayloadAttestation`

### Phase 4: Container Updates
14. Update `BeaconBlockBody<Gloas>` superstruct
15. Update `BeaconState<Gloas>` superstruct
16. Add Domain::BeaconBuilder

### Phase 5: Utilities
17. Helper methods on BeaconState: `get_builder()`, `get_pending_balance_to_withdraw_for_builder()`
18. Fork name helpers: `fork_name.gloas_enabled()`
19. Update fork versioning in all relevant places

## Files to Create/Modify

### New files to create:
```
consensus/types/src/builder/mod.rs
consensus/types/src/builder/builder.rs
consensus/types/src/builder/builder_pending_payment.rs
consensus/types/src/builder/builder_pending_withdrawal.rs
consensus/types/src/attestation/payload_attestation_data.rs
consensus/types/src/attestation/payload_attestation_message.rs
consensus/types/src/attestation/payload_attestation.rs
consensus/types/src/attestation/indexed_payload_attestation.rs
consensus/types/src/execution_payload_bid.rs
consensus/types/src/signed_execution_payload_bid.rs
consensus/types/src/execution_payload_envelope.rs (may exist)
consensus/types/src/signed_execution_payload_envelope.rs (may exist)
```

### Files to modify:
```
consensus/types/src/lib.rs - add exports
consensus/types/src/beacon_block_body.rs - add gloas variant
consensus/types/src/beacon_state.rs - add gloas fields
consensus/types/src/chain_spec.rs - add constants
consensus/types/src/application_domain.rs - add BeaconBuilder domain
consensus/types/src/eth_spec.rs - add gloas spec config
consensus/types/src/fork_name.rs - add gloas_enabled() helper
```

## Testing Strategy

For each new type:
1. Write SSZ encode/decode round-trip test
2. Write tree_hash test
3. Write default value test
4. Test superstruct transitions (e.g., Fulu -> Gloas beacon state upgrade)

Use upstream PR as reference for expected serialization.

## Spec References

Types defined in:
- https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md
- https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/fork-choice.md
- https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/validator.md

Upstream implementation:
- PR #7923 (a39e99155) - original types PR
- PR #8688 (21cabba1a) - spec 1.7.0-alpha.1 updates

## Decision: Start Small

Given the scale (52 files touched in upstream), implement incrementally:

1. Start with Builder types (simple structs, no superstruct)
2. Add ExecutionPayloadBid types (moderate complexity)
3. Test thoroughly with unit tests
4. Then tackle BeaconState/BeaconBlockBody superstruct updates (high complexity)
5. Commit after each logical group

**Estimated effort**: 4-6 work cycles (each cycle = 30-60 min of focused work + commit)

## Next Steps

1. Read `Builder` type from upstream: `git show a39e99155:consensus/types/src/builder/builder.rs`
2. Implement Builder + BuilderPendingPayment + BuilderPendingWithdrawal
3. Add unit tests
4. Commit: "types: implement gloas builder registry types"
5. Continue with execution payload bid types
