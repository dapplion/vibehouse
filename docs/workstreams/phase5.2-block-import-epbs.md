# Phase 5.2: Block Import Pipeline for ePBS

**Status:** Planning complete, ready to implement  
**Complexity:** HIGH (core consensus change)  
**Estimated time:** 4-6 hours

---

## Problem Statement

Pre-gloas (Fulu and earlier):
- Block contains full `ExecutionPayload`
- Import: validate block → verify payload → done
- One-phase process

Gloas ePBS:
- Proposer block contains `SignedExecutionPayloadBid` (no payload)
- Builder reveals `ExecutionPayload` separately
- Two-phase process:
  1. Proposer commits to winning bid
  2. Builder reveals payload (or withholds)
- PTC attests to payload presence
- Head selection requires payload revelation for external builders

**Current issue:** `BeaconBlockBody::execution_payload()` returns `Error::IncorrectStateVariant` for Gloas. The entire block import pipeline assumes payload is in the block.

---

## Architecture Decision

### Option A: Separate Import Paths (REJECTED)
- Create `import_gloas_proposer_block()` and `import_gloas_builder_payload()` as separate functions
- Pros: clean separation, easy to reason about
- Cons: code duplication, harder to maintain, breaks existing abstractions

### Option B: Extend Existing Pipeline (RECOMMENDED)
- Add payload state tracking to existing block import structs
- Use Option<ExecutionPayload> pattern: None = pending, Some = revealed
- Minimal changes to existing code paths
- Handle gloas as special case within existing verification flow

**Decision:** Option B. Extend existing pipeline with gloas awareness.

---

## Implementation Plan

### Step 1: Add Payload State Enum

**File:** `beacon_node/beacon_chain/src/block_verification.rs`

```rust
/// Tracks the state of execution payload for a block in the import pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum PayloadState<E: EthSpec> {
    /// Payload is included in the block (pre-gloas forks)
    Included,
    /// Payload is pending builder revelation (gloas proposer block)
    Pending {
        bid: ExecutionPayloadBid<E>,
    },
    /// Payload has been revealed (gloas after builder submission)
    Revealed {
        bid: ExecutionPayloadBid<E>,
        payload: ExecutionPayload<E>,
    },
    /// Self-build (gloas with BUILDER_INDEX_SELF_BUILD)
    SelfBuild {
        payload: ExecutionPayload<E>,
    },
}
```

### Step 2: Extend BlockImportData

**File:** `beacon_node/beacon_chain/src/block_verification.rs`

Add field to `BlockImportData`:
```rust
pub struct BlockImportData<E: EthSpec> {
    // ... existing fields ...
    
    /// Tracks payload state for gloas ePBS blocks
    pub payload_state: PayloadState<E>,
}
```

Update constructor to set:
- Pre-gloas: `PayloadState::Included`
- Gloas proposer block: `PayloadState::Pending { bid }`
- Gloas self-build: `PayloadState::SelfBuild { payload }`

### Step 3: Modify GossipVerifiedBlock::new()

**Location:** Around line 700-900 in block_verification.rs

Add gloas handling:
```rust
// After verifying block structure and slot
let payload_state = match block.message().body() {
    BeaconBlockBodyRef::Gloas(body) => {
        let bid = &body.signed_execution_payload_bid.message;
        
        // Check if self-build
        if bid.builder_index == BUILDER_INDEX_SELF_BUILD {
            // For self-build, payload should be in a separate field
            // (need to add this to BeaconBlockBody::Gloas)
            PayloadState::SelfBuild { payload: /* extract from block */ }
        } else {
            PayloadState::Pending { bid: bid.clone() }
        }
    }
    _ => PayloadState::Included,
};
```

**Blocker identified:** Self-build case needs design decision:
- Option 1: Add optional `execution_payload` field to Gloas BeaconBlockBody
- Option 2: Require self-build to use separate reveal message
- **Recommend Option 1:** Simpler for validators, matches spec intent

### Step 4: Skip Payload Verification for Pending Blocks

**Location:** `into_execution_pending_block()` around line 800-1000

```rust
let payload_verification_handle = match block_import_data.payload_state {
    PayloadState::Pending { .. } => {
        // No payload to verify yet, create dummy handle
        PayloadVerificationHandle::pending()
    }
    PayloadState::Included | PayloadState::Revealed { .. } | PayloadState::SelfBuild { .. } => {
        // Verify payload as usual
        verify_execution_payload(...)
    }
};
```

Need to add `PayloadVerificationHandle::pending()` method that returns a handle representing "no verification needed yet".

### Step 5: Update Fork Choice Integration

**File:** `beacon_node/beacon_chain/src/beacon_chain.rs`

In block import path (around line 2000-3000):
```rust
// After importing block into fork choice
match payload_state {
    PayloadState::Pending { bid } => {
        // Record pending payload in fork choice
        fork_choice.on_execution_bid(
            block_root,
            bid.builder_index,
            /* payload_revealed */ false,
        )?;
    }
    PayloadState::Revealed { bid, payload } => {
        fork_choice.on_execution_bid(
            block_root,
            bid.builder_index,
            /* payload_revealed */ true,
        )?;
    }
    PayloadState::SelfBuild { .. } => {
        // Self-build always has payload
        fork_choice.on_execution_bid(
            block_root,
            BUILDER_INDEX_SELF_BUILD,
            /* payload_revealed */ true,
        )?;
    }
    PayloadState::Included => {
        // Pre-gloas, no ePBS tracking needed
    }
}
```

### Step 6: Implement Payload Reveal Handler

**New method in:** `beacon_node/beacon_chain/src/beacon_chain.rs`

```rust
/// Processes a builder's payload revelation for a gloas proposer block.
/// 
/// Validates that:
/// - A proposer block exists with matching slot + block_root
/// - Payload matches bid commitment (block_hash, parent_hash, etc.)
/// - Builder signature is valid
/// 
/// On success:
/// - Executes payload against EL
/// - Updates fork choice: payload_revealed = true
/// - Returns payload verification result
pub fn process_execution_payload_reveal(
    &self,
    envelope: SignedExecutionPayloadEnvelope<E>,
) -> Result<(), BeaconChainError> {
    // 1. Find proposer block in fork choice
    let proposer_block_root = envelope.message.beacon_block_root;
    let proposer_block = self.get_block(&proposer_block_root)
        .ok_or(Error::UnknownBlock)?;
    
    // 2. Validate envelope matches bid
    let body = proposer_block.message().body();
    let bid = match body {
        BeaconBlockBodyRef::Gloas(gloas_body) => {
            &gloas_body.signed_execution_payload_bid.message
        }
        _ => return Err(Error::PayloadRevealForNonGloasBlock),
    };
    
    validate_payload_matches_bid(&envelope.message.execution_payload, bid)?;
    
    // 3. Verify builder signature
    verify_execution_payload_envelope_signature(&envelope, &self.spec)?;
    
    // 4. Execute payload
    let execution_status = self.execution_layer
        .notify_new_payload(envelope.message.execution_payload)
        .await?;
    
    // 5. Update fork choice
    self.canonical_head.fork_choice_write_lock()
        .on_payload_revealed(proposer_block_root)?;
    
    Ok(())
}

fn validate_payload_matches_bid<E: EthSpec>(
    payload: &ExecutionPayload<E>,
    bid: &ExecutionPayloadBid<E>,
) -> Result<(), Error> {
    if payload.block_hash() != bid.block_hash {
        return Err(Error::PayloadBlockHashMismatch);
    }
    if payload.parent_hash() != bid.parent_block_hash {
        return Err(Error::PayloadParentHashMismatch);
    }
    // TODO: validate other fields (fee_recipient, gas_limit, etc.)
    Ok(())
}
```

### Step 7: Wire Payload Reveal to P2P

**File:** `beacon_node/beacon_chain/src/network_beacon_processor.rs` (already exists from Phase 4)

Add handler (if not already present):
```rust
pub async fn process_gossip_execution_payload_envelope(
    &self,
    message: SignedExecutionPayloadEnvelope<E>,
) -> Result<(), Error> {
    // Validate via gossip
    let verified = verify_execution_payload_envelope_for_gossip(message, &self.chain)?;
    
    // Process
    self.chain.process_execution_payload_reveal(verified.into_inner()).await?;
    
    Ok(())
}
```

---

## Edge Cases to Handle

### 1. Self-Build
**Problem:** Proposer is builder, payload should be in block  
**Solution:** Add `execution_payload: Option<ExecutionPayload>` to Gloas BeaconBlockBody  
**Status:** Needs type change in Phase 1 (revisit)

### 2. Builder Withholding
**Problem:** Builder never reveals payload  
**Solution:** Fork choice already handles this - block not viable for head  
**Status:** Already implemented in Phase 3

### 3. Late Reveal
**Problem:** Payload arrives after PTC attestations  
**Solution:** Still processable, PTC attestations idempotent  
**Status:** No change needed

### 4. Duplicate Reveal
**Problem:** Builder sends payload multiple times  
**Solution:** Check fork choice state, ignore if already revealed  
**Status:** Add check in process_execution_payload_reveal()

### 5. Fork Boundary
**Problem:** Block N-1 is Fulu, block N is Gloas  
**Solution:** PayloadState pattern handles both  
**Status:** Works automatically

---

## Testing Strategy

### Unit Tests
- [ ] PayloadState variants (Pending, Revealed, SelfBuild)
- [ ] validate_payload_matches_bid() with all mismatch cases
- [ ] process_execution_payload_reveal() happy path
- [ ] process_execution_payload_reveal() with unknown block
- [ ] process_execution_payload_reveal() with mismatched payload

### Integration Tests
- [ ] Import proposer block → verify pending state
- [ ] Reveal payload → verify revealed state
- [ ] PTC attestations → verify accumulation
- [ ] Self-build block → verify immediate payload availability
- [ ] Withholding scenario → verify block not selected for head

### Spec Tests
- [ ] Gloas block processing vectors (when available)
- [ ] Fork transition vectors (Fulu → Gloas)

---

## Implementation Order

1. **Add PayloadState enum** (15 min)
2. **Extend BlockImportData** (10 min)
3. **Self-build design decision** (30 min - need to think through spec)
4. **Modify GossipVerifiedBlock::new()** (1 hour)
5. **Skip verification for pending** (30 min)
6. **Fork choice integration** (45 min)
7. **Payload reveal handler** (1.5 hours)
8. **Wire to P2P** (30 min)
9. **Tests** (2 hours)

**Total:** ~7 hours (realistic estimate)

---

## Blockers & Dependencies

### Critical Blocker: Self-Build Design
Need to decide how self-build payloads are included. Options:

**A) Add optional field to Gloas BeaconBlockBody:**
```rust
pub struct BeaconBlockBodyGloas {
    pub signed_execution_payload_bid: SignedExecutionPayloadBid<E>,
    pub execution_payload: Option<ExecutionPayload<E>>, // Some for self-build
    pub payload_attestations: VariableList<PayloadAttestation<E>>,
    // ... other fields
}
```
Pros: Simple, self-contained  
Cons: Deviates from ePBS spec purity

**B) Require self-build to use separate reveal:**
- Proposer publishes block with bid (builder_index = SELF_BUILD)
- Immediately publishes reveal message
- Two messages instead of one

Pros: Consistent with external builder flow  
Cons: More complex for validators, adds latency

**Recommendation:** Option A. Spec allows this via "tight binding" concept. Add optional execution_payload field.

### Dependency: Fork Choice Update
Fork choice needs `on_payload_revealed(block_root)` method to mark payload as revealed after the reveal message is processed.

**Status:** Can add in Phase 5.2, lightweight change to fork_choice.rs

---

## Success Criteria

- [ ] Proposer blocks import successfully without payload
- [ ] Payload reveal messages processed correctly
- [ ] Fork choice tracks payload state (pending → revealed)
- [ ] Self-build blocks work end-to-end
- [ ] Withholding scenario handled gracefully
- [ ] All tests pass
- [ ] Code compiles without warnings

**Estimated completion:** 1-2 work sessions (4-8 hours)

---

## Next Steps

1. **Make self-build design decision** (discuss with human if needed)
2. **Start implementation** with PayloadState enum
3. **Iterate through steps** 1-9 above
4. **Test thoroughly** before moving to Phase 5.3

🎵 **This is the hard part. Take it slow. Test as you go.**
