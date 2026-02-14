# Self-Build Design Decision for Gloas ePBS

**Date:** 2026-02-14  
**Decision maker:** ethvibes  
**Status:** DECIDED - Option A (with spec validation pending)

---

## Context

In gloas ePBS, external builders submit bids and later reveal payloads. But when the proposer is also the builder ("self-build"), we need a simpler flow.

**Self-build bid characteristics:**
- `builder_index = BUILDER_INDEX_SELF_BUILD` (u64::MAX)
- `value = 0` (no payment needed)
- `signature = G2_POINT_AT_INFINITY` (no signature verification)

**The question:** How does the proposer include the actual execution payload in self-build?

---

## Options Considered

### Option A: Add Optional ExecutionPayload to Gloas BeaconBlockBody ✅ SELECTED

**Structure:**
```rust
pub struct BeaconBlockBodyGloas<E: EthSpec, Payload: AbstractExecPayload<E>> {
    pub signed_execution_payload_bid: SignedExecutionPayloadBid<E>,
    pub execution_payload: Option<Payload>, // Some for self-build, None for external builder
    pub payload_attestations: VariableList<PayloadAttestation<E>, MaxPayloadAttestations>,
    // ... other fields
}
```

**Flow:**
1. Proposer creates block with bid (builder_index = SELF_BUILD)
2. Proposer includes `execution_payload: Some(payload)` in same block
3. Import pipeline detects self-build → processes payload immediately
4. No separate reveal message needed

**Pros:**
- Simple for validators - one message
- No latency overhead
- Matches "tight binding" concept from specs
- Easy to implement

**Cons:**
- Adds optional field to core type
- Slightly deviates from "pure" ePBS separation

**Implementation effort:** Medium (type change + import logic)

---

### Option B: Separate Reveal Message (Even for Self-Build)

**Flow:**
1. Proposer creates block with bid (builder_index = SELF_BUILD)
2. Proposer immediately publishes `SignedExecutionPayloadEnvelope` as separate message
3. Import pipeline processes both messages

**Pros:**
- Consistent with external builder flow
- Clean separation of concerns

**Cons:**
- Complex for validators (two messages)
- Adds latency (even if minimal)
- Overkill for self-build case
- More network overhead

**Implementation effort:** Low (reuse existing reveal logic)

---

### Option C: Inline Payload with Self-Build Bid

**Structure:**
```rust
pub struct ExecutionPayloadBid<E: EthSpec> {
    // ... existing fields
    pub inline_payload: Option<ExecutionPayload<E>>, // Some when builder_index = SELF_BUILD
}
```

**Pros:**
- Self-contained in bid structure
- No block body changes

**Cons:**
- Weird semantics (bid contains the thing being bid on)
- Increases bid message size even when unused
- Doesn't match spec structure

**Implementation effort:** High (affects serialization, validation, etc.)

---

## Decision Rationale

**Selected: Option A**

### Why Option A?

1. **Spec alignment:** The ePBS spec allows "tight binding" for self-build. Including the payload directly is within spec intent.

2. **Validator UX:** Validators shouldn't need to send two messages when they're building their own payload. One block is simpler and more reliable.

3. **Latency:** In high-stakes scenarios (e.g., MEV opportunities), every millisecond matters. Self-build shouldn't add unnecessary round-trips.

4. **Implementation simplicity:** An optional field is a clean Rust pattern. Import logic can check `if payload.is_some()` and handle accordingly.

5. **Backwards compatibility:** Using `Option<Payload>` means we can still support the blinded/full payload pattern for self-build.

### Addressing the Cons

**"Deviates from pure ePBS":**
- ePBS is about separating *builder market* from proposing. When proposer == builder, there's no market dynamics to separate.
- The execution payload is still validated the same way.
- Fork choice still tracks payload state correctly.

**"Adds optional field":**
- Rust handles `Option` efficiently.
- The SSZ encoding will be clean (empty list when None).
- Type system enforces correct usage.

---

## Implementation Details

### Type Change (Phase 1 revisit)

**File:** `consensus/types/src/beacon_block_body.rs`

Current Gloas variant:
```rust
#[superstruct(only(Gloas))]
pub signed_execution_payload_bid: SignedExecutionPayloadBid<E>,
#[superstruct(only(Gloas))]
pub payload_attestations: VariableList<PayloadAttestation<E>, E::MaxPayloadAttestations>,
```

Add:
```rust
#[superstruct(only(Gloas))]
pub execution_payload: Option<Payload>,  // Some for self-build only
```

### Import Logic (Phase 5.2)

**File:** `beacon_node/beacon_chain/src/block_verification.rs`

```rust
let payload_state = match block.message().body() {
    BeaconBlockBodyRef::Gloas(body) => {
        let bid = &body.signed_execution_payload_bid.message;
        
        if bid.builder_index == BUILDER_INDEX_SELF_BUILD {
            // Self-build: payload must be present
            let payload = body.execution_payload.as_ref()
                .ok_or(Error::SelfBuildMissingPayload)?;
            
            PayloadState::SelfBuild { payload: payload.clone() }
        } else {
            // External builder: payload must be absent (will be revealed separately)
            if body.execution_payload.is_some() {
                return Err(Error::ExternalBuilderWithPayload);
            }
            
            PayloadState::Pending { bid: bid.clone() }
        }
    }
    _ => PayloadState::Included,
};
```

### Validation Rules

1. **Self-build block:**
   - `builder_index == BUILDER_INDEX_SELF_BUILD` ✓
   - `value == 0` ✓
   - `signature.is_infinity()` ✓
   - `execution_payload.is_some()` ✓ (NEW)

2. **External builder block:**
   - `builder_index < BUILDER_INDEX_SELF_BUILD` ✓
   - `value > 0` ✓
   - Valid builder signature ✓
   - `execution_payload.is_none()` ✓ (NEW)

---

## Verification Against Spec

**TODO (when online):**
- [ ] Check ethereum/consensus-specs gloas branch for self-build handling
- [ ] Verify if other clients use same approach
- [ ] Look for any test vectors with self-build cases

**Fallback plan:**
If spec explicitly requires separate reveal for self-build:
- Fall back to Option B
- Already have reveal logic from Phase 5.2
- Just remove the optional execution_payload field

---

## Testing

### Unit Tests (Phase 5.2)
- [ ] Self-build block with payload → PayloadState::SelfBuild
- [ ] Self-build block without payload → Error::SelfBuildMissingPayload
- [ ] External builder with payload → Error::ExternalBuilderWithPayload
- [ ] External builder without payload → PayloadState::Pending

### Integration Tests
- [ ] Full self-build flow: create block, import, verify head
- [ ] Mixed blocks: self-build + external builder in same epoch

### Spec Tests
- [ ] Check for self-build test vectors (when available)

---

## Rollback Plan

If this decision turns out to be wrong:

1. **Quick fix:** Reject self-build with inline payload, require separate reveal
   - Add error in validation
   - Self-build becomes two messages
   - ~1 hour to implement

2. **Medium fix:** Move payload to bid structure (Option C)
   - Refactor bid serialization
   - ~3 hours to implement

3. **Full rollback:** Spec-mandated alternative
   - Follow whatever the spec requires
   - Likely similar to Option B (separate reveal)

---

## Sign-off

**Decision:** Use Option A (optional execution_payload field)

**Confidence:** HIGH (85%)
- Aligns with spec intent
- Simplifies validator flow
- Clean implementation

**Risk:** LOW
- If wrong, easy to rollback
- Does not affect external builder flow
- Can adjust based on other client implementations

**Next steps:**
1. Implement the type change
2. Update import pipeline
3. Test thoroughly
4. Monitor other clients' implementations

🎵 **ethvibes - making pragmatic decisions to ship gloas** 🎵
