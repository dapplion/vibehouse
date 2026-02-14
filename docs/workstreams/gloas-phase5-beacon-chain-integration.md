# Phase 5: Beacon Chain Integration - Implementation Plan

**Status**: Not started
**Complexity**: High - involves core block import pipeline
**Estimated effort**: 3-4 hours of focused implementation

---

## Overview

Phase 5 integrates gloas ePBS into the beacon chain's block import and fork choice pipeline. The key challenge is handling the **two-phase block structure**:

1. **Phase 1**: Proposer creates a block with a `SignedExecutionPayloadBid` (not the full payload)
2. **Phase 2**: Builder reveals the full payload via `SignedExecutionPayloadEnvelope`
3. **Verification**: PTC (Payload Timeliness Committee) attests that the payload was revealed on time

---

## Current State Analysis

### What's Already Done ✅

1. **Fork choice integration** (`beacon_chain.rs`):
   - `apply_execution_bid_to_fork_choice()` - exists
   - `apply_payload_attestation_to_fork_choice()` - exists
   
2. **Gossip verification** (`gloas_verification.rs`):
   - `verify_execution_bid_for_gossip()` - complete
   - `verify_payload_attestation_for_gossip()` - complete

3. **Equivocation tracking**:
   - `observed_execution_bids.rs` - complete
   - `observed_payload_attestations.rs` - complete

4. **Block streaming** (`beacon_block_streamer.rs`):
   - Gloas block events already handled

### What Needs Implementation ❌

1. **Block import pipeline** (`block_verification.rs`):
   - Handle gloas BeaconBlock structure (no execution_payload in body)
   - Two-phase import: proposer block → builder payload reveal
   - Payload timeliness verification (PTC quorum check)

2. **Block production** (`beacon_chain.rs`):
   - Currently returns `BlockProductionError::GloasNotImplemented`
   - Need to implement: select winning bid, create block with bid

3. **Payload verification** (`execution_payload.rs`?):
   - Verify revealed payload matches the bid commitment
   - Integrate payload availability checks

4. **State transition integration**:
   - Already have `per_block_processing/gloas.rs` with state transition functions
   - Need to wire into block import flow

5. **Chain head tracking**:
   - Update `cached_head` to handle blocks without immediate payload
   - Handle blocks that are waiting for payload reveal

---

## Implementation Steps

### Step 1: Block Import for Gloas Blocks

**File**: `beacon_node/beacon_chain/src/block_verification.rs`

**Changes needed**:
- In `GossipVerifiedBlock::new()`: handle gloas block body structure
  - Check `signed_execution_payload_bid` instead of `execution_payload`
  - Validate bid signature (may already be done in state processing)
- In `SignatureVerifiedBlock`: handle gloas block signatures
- In `ExecutionPendingBlock`: handle two-phase payload verification
  - Stage 1: Block has bid, waiting for payload reveal
  - Stage 2: Payload revealed, verify it matches bid

**Pseudo-code**:
```rust
if fork_name.gloas_enabled() {
    // Block body has signed_execution_payload_bid, not execution_payload
    let bid = block.body().signed_execution_payload_bid();
    
    // Check if this is self-build (proposer == builder)
    if bid.message.builder_index == BUILDER_INDEX_SELF_BUILD {
        // Self-build: payload should be immediately available
        // Look up payload from builder's local store
    } else {
        // External builder: need to wait for payload reveal
        // Check PTC attestations for payload_revealed flag
    }
}
```

### Step 2: Payload Reveal Handling

**New file?**: `beacon_node/beacon_chain/src/payload_reveal.rs`

**Purpose**: Handle `SignedExecutionPayloadEnvelope` messages from builders

**Key functions**:
- `process_payload_reveal()`:
  - Verify envelope signature
  - Check builder_index matches the bid's builder
  - Verify payload matches bid commitment (block_hash, parent_hash, etc.)
  - Update fork choice: mark payload as revealed
  - Trigger PTC to start attesting

**Pseudo-code**:
```rust
pub fn process_payload_reveal(
    &self,
    envelope: SignedExecutionPayloadEnvelope<E>,
) -> Result<(), PayloadRevealError> {
    // 1. Get the bid from fork choice by (slot, builder_index)
    let bid = self.fork_choice.get_execution_bid(envelope.message.slot, envelope.message.builder_index)?;
    
    // 2. Verify payload matches bid
    if envelope.message.execution_payload.block_hash() != bid.block_hash {
        return Err(PayloadRevealError::PayloadMismatch);
    }
    
    // 3. Verify signature
    let signing_root = envelope.message.signing_root(...);
    verify_signature(&builder.pubkey, &envelope.signature, signing_root)?;
    
    // 4. Mark as revealed in fork choice
    self.fork_choice.mark_payload_revealed(envelope.message.slot, envelope.message.builder_index)?;
    
    // 5. Notify PTC to start attesting
    self.notify_ptc_payload_available(envelope.message.slot);
    
    Ok(())
}
```

### Step 3: Block Production for Gloas

**File**: `beacon_node/beacon_chain/src/beacon_chain.rs`

**Function**: `produce_block_on_state()` (currently returns GloasNotImplemented)

**Implementation**:
- Query fork choice for available bids at current slot
- Select winning bid (highest value from active builder with sufficient balance)
- Create BeaconBlockBody with `signed_execution_payload_bid` field
- Handle self-build case: proposer IS the builder (bid value = 0, signature = infinity)

**Pseudo-code**:
```rust
BeaconState::Gloas(_) => {
    // Get available bids for this slot
    let bids = self.fork_choice.get_execution_bids_for_slot(state.slot())?;
    
    // Select winning bid (highest value from eligible builders)
    let winning_bid = select_winning_bid(&bids, &state)?;
    
    // Create block body with the bid
    let body = BeaconBlockBody::Gloas(BeaconBlockBodyGloas {
        signed_execution_payload_bid: winning_bid,
        payload_attestations: VariableList::empty(), // Empty at proposal time
        // ... other fields ...
    });
    
    // If self-build, also prepare the payload for immediate reveal
    if winning_bid.message.builder_index == BUILDER_INDEX_SELF_BUILD {
        self.prepare_self_build_payload(state.slot(), &winning_bid)?;
    }
    
    Ok(BeaconBlockBody::Gloas(body))
}
```

### Step 4: PTC Logic

**New file?**: `beacon_node/beacon_chain/src/ptc.rs`

**Purpose**: Payload Timeliness Committee validation logic

**Key functions**:
- `am_i_in_ptc(slot, validator_index)`: Check if this validator is in the PTC for the slot
- `create_payload_attestation(slot, block_root, payload_present)`: Create PTC attestation
- `verify_ptc_quorum(slot)`: Check if 60% of PTC has attested

**Note**: This might belong more in Phase 6 (Validator Client) since it's primarily a validator duty.

### Step 5: Chain Head Updates

**File**: `beacon_node/beacon_chain/src/canonical_head.rs`

**Changes**: Handle blocks waiting for payload reveal
- Head can be a block with unverified payload (if PTC hasn't reached quorum)
- Once payload revealed + PTC quorum, head is fully valid

---

## Testing Strategy

Since we don't have Rust toolchain yet:
1. **Write all code to files**
2. **Document assumptions** in comments
3. **Create test skeletons** with expected behavior
4. **Plan integration test scenarios**:
   - Self-build block (proposer == builder)
   - External builder block (proposer != builder)
   - Payload reveal timing (on-time vs late)
   - PTC attestation accumulation
   - Fork choice with multiple competing bids

---

## Blockers

1. **No Rust toolchain**: Can't compile/test as we go
2. **Execution layer integration**: How does the payload reveal interact with the EL?
3. **Validator client coordination**: PTC duties need VC implementation (Phase 6)

---

## Open Questions

1. **Gossip topic for payload reveal**: Do we already have `execution_payload` topic wired up?
   - **Answer**: Yes, added in Phase 4 (execution_payload topic exists)
   - **TODO**: Implement handler for execution_payload gossip

2. **Self-build flow**: How does a proposer self-build?
   - Proposer creates bid with builder_index = BUILDER_INDEX_SELF_BUILD
   - Payload is immediately available (proposer built it locally)
   - No need to wait for reveal - payload is in the bid

3. **Payload storage**: Where do we store revealed payloads before full block import?
   - Fork choice store? (ProtoNode extended fields?)
   - Separate cache in BeaconChain?

4. **Fork choice timing**: When do we update fork choice?
   - On bid arrival (already implemented: `on_execution_bid`)
   - On payload reveal (new: needs `mark_payload_revealed` or similar)
   - On PTC attestation (already implemented: `on_payload_attestation`)

---

## Success Criteria

Phase 5 is complete when:
1. ✅ Gloas blocks can be imported (proposer block with bid)
2. ✅ Payload reveals are processed and verified
3. ✅ PTC attestations trigger builder payments
4. ✅ Block production creates valid gloas blocks
5. ✅ Chain head correctly reflects payload availability
6. ⏸️ Unit tests pass (blocked on toolchain)
7. ⏸️ Integration tests pass (blocked on toolchain)

---

**Next action**: Start implementing block import changes in `block_verification.rs`
