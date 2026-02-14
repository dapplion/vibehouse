# Gloas Beacon Processor Integration Plan

**Status**: Ready to implement (blocked on Rust toolchain)
**Date**: 2026-02-14
**Phase**: 4 Step 4 (P2P Networking)

## Overview

Integrate gloas ePBS gossip messages into the beacon processor pipeline. This connects the gossip validation layer (already implemented) to fork choice handlers.

## Message Flow

```
Gossip Message → Validation (gloas_verification.rs) → Beacon Processor (Work enum) → Fork Choice Handler
```

## Implementation Tasks

### 1. Add Work Variants

**File**: `beacon_node/beacon_processor/src/lib.rs`

Add 3 new variants to `Work<E>` enum:

```rust
pub enum Work<E: EthSpec> {
    // ... existing variants ...
    
    GossipExecutionBid(BlockingFn),
    GossipExecutionPayload(BlockingFn),
    GossipPayloadAttestation(BlockingFn),
}
```

Add corresponding `WorkType` variants:

```rust
pub enum WorkType {
    // ... existing variants ...
    
    GossipExecutionBid,
    GossipExecutionPayload,
    GossipPayloadAttestation,
}
```

### 2. Create Process Methods

**File**: `beacon_node/network/src/network_beacon_processor/gossip_methods.rs`

#### 2.1 Process Execution Bid

```rust
pub fn process_gossip_execution_bid(
    self,
    message_id: MessageId,
    peer_id: PeerId,
    bid: SignedExecutionPayloadBid<T::EthSpec>,
    seen_timestamp: Duration,
) {
    // 1. Validate using VerifiedExecutionBid::verify_for_gossip()
    // 2. Call chain.on_execution_bid(verified_bid)
    // 3. Propagate if valid (MessageAcceptance::Accept)
    // 4. Track in metrics
}
```

**Dependencies**:
- Import `VerifiedExecutionBid` from `beacon_chain::gloas_verification`
- Import `SignedExecutionPayloadBid` from `types`

#### 2.2 Process Execution Payload

```rust
pub fn process_gossip_execution_payload(
    self,
    message_id: MessageId,
    peer_id: PeerId,
    payload: SignedExecutionPayloadEnvelope<T::EthSpec>,
    seen_timestamp: Duration,
) {
    // 1. Validate envelope signature
    // 2. Check payload matches winning bid
    // 3. Call chain.process_execution_payload_reveal()
    // 4. Propagate if valid
}
```

**Note**: Payload reveal processing might be complex - needs coordination with block import.

#### 2.3 Process Payload Attestation

```rust
pub fn process_gossip_payload_attestation(
    self,
    message_id: MessageId,
    peer_id: PeerId,
    attestation: PayloadAttestation<T::EthSpec>,
    seen_timestamp: Duration,
) {
    // 1. Validate using VerifiedPayloadAttestation::verify_for_gossip()
    // 2. Call chain.on_payload_attestation(verified_attestation)
    // 3. Propagate if valid
    // 4. Track PTC participation metrics
}
```

**Dependencies**:
- Import `VerifiedPayloadAttestation` from `beacon_chain::gloas_verification`
- Import `PayloadAttestation` from `types`

### 3. Wire Up Gossip Handlers

**File**: `beacon_node/network/src/router/processor.rs` (or similar)

Add message routing:

```rust
match gossip_topic {
    GossipKind::ExecutionBid => {
        let bid = SignedExecutionPayloadBid::from_ssz_bytes(msg)?;
        processor.process_gossip_execution_bid(msg_id, peer_id, bid, seen_timestamp);
    }
    GossipKind::ExecutionPayload => {
        let payload = SignedExecutionPayloadEnvelope::from_ssz_bytes(msg)?;
        processor.process_gossip_execution_payload(msg_id, peer_id, payload, seen_timestamp);
    }
    GossipKind::PayloadAttestation => {
        let attestation = PayloadAttestation::from_ssz_bytes(msg)?;
        processor.process_gossip_payload_attestation(msg_id, peer_id, attestation, seen_timestamp);
    }
    // ... existing cases ...
}
```

### 4. Add Metrics

**File**: `beacon_node/network/src/metrics.rs`

Add counters and histograms:

```rust
pub static GOSSIP_EXECUTION_BID: LazyLock<Result<IntCounterVec>> = ...;
pub static GOSSIP_EXECUTION_PAYLOAD: LazyLock<Result<IntCounterVec>> = ...;
pub static GOSSIP_PAYLOAD_ATTESTATION: LazyLock<Result<IntCounterVec>> = ...;

pub static GOSSIP_EXECUTION_BID_VERIFICATION_TIME: LazyLock<Result<Histogram>> = ...;
pub static GOSSIP_PAYLOAD_ATTESTATION_VERIFICATION_TIME: LazyLock<Result<Histogram>> = ...;
```

Track:
- Received count
- Valid/invalid count
- Equivocation count
- Verification duration
- PTC quorum progress

## Integration Points

### Fork Choice Connection

The `process_gossip_*` methods must call the fork choice handlers we already implemented:

- `chain.on_execution_bid()` → updates ProtoNode.builder_index
- `chain.on_payload_attestation()` → accumulates PTC weight

These methods are in `beacon_node/beacon_chain/src/fork_choice.rs` (or similar).

### Equivocation Handling

When `VerifiedExecutionBid::verify_for_gossip()` returns `ObservationOutcome::Equivocating`:

1. Log warning with details
2. **DO NOT** propagate the message (`MessageAcceptance::Reject`)
3. Apply peer penalty (`ReportSource::Gossipsub`, `PeerAction::LowToleranceError`)
4. Mark builder/validator as equivocating in fork choice store

### Error Handling

Follow existing patterns for different error categories:

- **Invalid signature**: Reject + peer penalty
- **Unknown parent**: Queue for reprocessing
- **Future slot**: Queue for delayed processing
- **Equivocation**: Reject + mark + penalty

## Testing Requirements

### Unit Tests

1. Test each `process_gossip_*` method with:
   - Valid message → accept + propagate
   - Invalid signature → reject + penalty
   - Equivocation → reject + mark
   - Unknown parent → reprocess queue

### Integration Tests

1. Full message flow: gossip → validation → fork choice → head update
2. PTC quorum: send 307 attestations → payload marked revealed
3. Builder payment: bid accepted → attestations accumulate → payment triggered

## Compilation Dependencies

This work requires:

1. ✅ Gossip validation types (`gloas_verification.rs`) - already done
2. ✅ Fork choice handlers (`on_execution_bid`, `on_payload_attestation`) - already done
3. ⏳ Beacon processor Work variants - this task
4. ⏳ Router wiring - this task
5. ⏳ Metrics - this task

## Blockers

- **Rust toolchain issue**: bitvec 0.17.4 incompatible with newer Rust
- Can't compile/test until toolchain is available
- All design work and code can be written, but not validated

## Next Steps (When Toolchain Available)

1. Add Work variants to lib.rs
2. Implement `process_gossip_execution_bid()`
3. Implement `process_gossip_payload_attestation()`
4. Wire up routing in processor.rs
5. Add metrics
6. Write unit tests
7. Run integration tests
8. Verify with `make test-ef`

## Estimated Effort

- **Work variants**: 30 min
- **Process methods**: 2-3 hours (includes error handling, metrics, tests)
- **Router wiring**: 1 hour
- **Metrics**: 30 min
- **Testing**: 2 hours

**Total**: ~6 hours of focused work once toolchain is available.

---

**Status**: Implementation plan complete. Ready to execute when compilation is possible.
