# Phase 5: Beacon Chain Integration - Implementation Plan

**Status:** Ready to start (Phase 4 complete)  
**Priority:** High (core consensus functionality)  
**Estimated effort:** 5-7 work sessions

---

## Overview

Phase 5 integrates the ePBS block processing pipeline into the beacon chain. This is where proposer blocks (with bids) and builder payloads come together.

**Key challenge:** ePBS introduces a two-phase block structure:
1. **Proposer block** - contains `SignedExecutionPayloadBid`, no full payload
2. **Builder payload reveal** - separate message with actual `ExecutionPayload`

The beacon chain must handle both phases and coordinate with fork choice.

---

## Implementation Tasks (6 items)

### 5.1 Wire up gloas types through beacon chain crate ✅ PARTIALLY DONE

**Status:** Processing methods exist (process_execution_bid, process_payload_attestation)

**Remaining:**
- [ ] Add gloas-specific imports to beacon_chain crate root
- [ ] Export new verification modules (execution_bid_verification, payload_attestation_verification)
- [ ] Ensure all gloas types accessible throughout beacon_chain

**Files:**
- `beacon_node/beacon_chain/src/lib.rs` - exports

**Estimated time:** 15 minutes

---

### 5.2 Update block import pipeline for ePBS ⚠️ COMPLEX

**Current flow (pre-gloas):**
1. Block arrives via gossip/RPC
2. Block contains full `ExecutionPayload`
3. Import validates + processes payload in one step

**New flow (gloas ePBS):**
1. **Proposer block** arrives with `SignedExecutionPayloadBid` (no payload)
2. Import validates bid, records block as "payload pending"
3. **Builder reveals** `ExecutionPayload` via separate gossip message
4. Import validates payload matches bid commitment, marks "payload revealed"
5. PTC attestations accumulate weight
6. Block becomes eligible for head once payload revealed + quorum

**Required changes:**
- [ ] Add block import state: `PayloadPending`, `PayloadRevealed`
- [ ] Modify `process_block()` to handle missing payload in gloas
- [ ] Implement payload reveal handler: `process_execution_payload_reveal()`
- [ ] Update payload validation: check against bid commitment (block_hash, parent_hash, etc.)
- [ ] Coordinate with fork choice: payload revelation updates `payload_revealed` flag
- [ ] Handle self-build case (BUILDER_INDEX_SELF_BUILD) - payload included in proposer block

**Files:**
- `beacon_node/beacon_chain/src/block_verification.rs` - core import logic
- `beacon_node/beacon_chain/src/beacon_chain.rs` - add process_execution_payload_reveal()
- `consensus/state_processing/src/per_block_processing/mod.rs` - gloas block processing flow

**Edge cases:**
- Missing payload (builder withholds) - handled by fork choice head selection
- Late payload reveal (after PTC attestations) - still processable
- Duplicate payload reveals - idempotent handling
- Self-build - payload in proposer block, skip reveal phase

**Estimated time:** 3-4 hours

---

### 5.3 Update fork choice store integration

**Current:** Fork choice store tracks blocks, attestations, slashings

**New (gloas):**
- [x] Store already tracks `builder_index`, `payload_revealed`, `ptc_weight` (ProtoNode)
- [ ] Ensure store persists execution bids (bid → block_root mapping)
- [ ] Add store queries: get_bids_for_slot(), get_payload_status(block_root)
- [ ] Update store pruning: remove old bids when finalized

**Files:**
- `consensus/fork_choice/src/fork_choice_store.rs`
- `beacon_node/beacon_chain/src/canonical_head.rs`

**Estimated time:** 1-2 hours

---

### 5.4 Handle two-phase block: proposer commits, builder reveals ⚠️ COMPLEX

**This is the heart of ePBS block processing.**

**Proposer block processing:**
1. Validate `SignedExecutionPayloadBid`:
   - Slot matches block slot
   - Builder exists and is active
   - Signature valid (DOMAIN_BEACON_BUILDER)
   - Bid amount ≤ builder balance
2. Record bid in state: `state.latest_execution_payload_bid = bid.message`
3. Mark block as "payload pending" in fork choice
4. **Do NOT process execution payload** (it's not included yet)

**Builder payload reveal processing:**
1. Receive `SignedExecutionPayloadEnvelope` via gossip
2. Validate envelope:
   - Slot + block_root match pending proposer block
   - Payload fields match bid commitment (block_hash, parent_hash, etc.)
   - Builder signature valid
3. Execute payload against EL (engine API)
4. Update fork choice: `payload_revealed = true`
5. Trigger PTC: validators see revealed payload, start attesting

**PTC attestation accumulation:**
- Already handled in Phase 4 (fork choice `on_payload_attestation`)
- Once quorum (307/512) reached → block eligible for head

**Files:**
- `beacon_node/beacon_chain/src/execution_payload_processing.rs` - new module for ePBS payload handling
- `consensus/state_processing/src/per_block_processing/gloas.rs` - extend with payload reveal logic

**Estimated time:** 4-5 hours

---

### 5.5 Implement payload timeliness committee (PTC) logic

**PTC duties (validator perspective - Phase 6):**
- 512 validators per slot selected deterministically
- Duty: attest to payload presence after builder reveals
- Already implemented: `get_ptc_committee()` in state_processing

**Beacon chain integration:**
- [ ] Add PTC duty scheduler (similar to attestation duties)
- [ ] Expose PTC committee via API (Phase 7)
- [ ] Handle PTC attestation publishing (Phase 6 - validator client)

**This mostly belongs in Phase 6 (validator client), but beacon chain needs:**
- [ ] Method: `get_ptc_duties(epoch, validator_indices)` → List of (slot, is_member)
- [ ] Integrate with existing duty scheduler

**Files:**
- `beacon_node/beacon_chain/src/validator_duties.rs`

**Estimated time:** 1 hour

---

### 5.6 Update chain head tracking for ePBS

**Current:** Head selection uses fork choice `get_head()` - already updated in Phase 3

**New considerations:**
- [ ] Ensure `node_is_viable_for_head()` enforcement works end-to-end
- [ ] Log warnings when best block has unconfirmed payload (withholding detected)
- [ ] Metrics: track payload revelation latency (block import → reveal)
- [ ] Ensure re-org handling works with two-phase blocks

**Files:**
- `beacon_node/beacon_chain/src/canonical_head.rs`
- Add metrics in `beacon_node/beacon_chain/src/metrics.rs`

**Metrics to add:**
- `beacon_payload_reveal_latency_seconds` - time from block import to payload reveal
- `beacon_payload_withholding_total` - count of blocks with missing payload at head selection
- `beacon_ptc_quorum_latency_seconds` - time from payload reveal to quorum

**Estimated time:** 1 hour

---

## Success Criteria

**Phase 5 complete when:**
1. Proposer blocks (with bids, no payload) import successfully
2. Builder payload reveals processed and validated against bid
3. Fork choice correctly tracks payload_revealed status
4. Head selection enforces payload revelation for external builders
5. Self-build blocks (BUILDER_INDEX_SELF_BUILD) process correctly
6. No regressions in existing (pre-gloas) block processing

**Testing (can't run yet, but document):**
- Unit tests for two-phase block flow
- Integration test: proposer block → payload reveal → PTC attestations → head
- Self-build test: proposer includes payload, skips reveal phase
- Withholding test: builder doesn't reveal, block not selected as head

---

## Phased Approach

**Session 1:** Task 5.1 + 5.3 (wire types + store integration) - 2 hours  
**Session 2:** Task 5.2 (block import pipeline - part 1) - 2 hours  
**Session 3:** Task 5.2 (block import pipeline - part 2) + 5.4 (two-phase logic) - 3 hours  
**Session 4:** Task 5.4 (payload reveal) - 2 hours  
**Session 5:** Task 5.5 (PTC duties) + 5.6 (head tracking + metrics) - 2 hours  

**Total estimated:** 11 hours across 5 sessions

---

## Dependencies

**Blocked on:**
- None - all prerequisites complete (Phases 1-4 ✅)

**Blocks:**
- Phase 6 (validator client) - needs PTC duties, block proposal flow
- Phase 7 (REST API) - needs block retrieval for two-phase blocks

---

## Risk Areas

**High complexity:**
- Two-phase block import (many edge cases)
- Payload commitment validation (must match bid exactly)
- Self-build vs external builder logic paths

**Potential bugs:**
- Race condition: payload reveal arrives before proposer block
- Equivocation: builder reveals conflicting payloads
- State corruption: bid not properly recorded before reveal

**Mitigation:**
- Write comprehensive doc comments explaining flow
- Add extensive logging at each step
- Validate invariants: bid must exist before payload reveal

---

## Next Actions

1. Start with Task 5.1 (wire up types) - easy warmup
2. Read existing block import code to understand flow
3. Sketch out state machine for two-phase blocks
4. Implement proposer block import (no payload)
5. Implement payload reveal handler
6. Test (once toolchain available)

**Ready to begin Phase 5.** 🎵
