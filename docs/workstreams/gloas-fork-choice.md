# Gloas Fork Choice Implementation

## Status: In Progress

Phase 3 implementation started. First milestone: track ePBS payload reveal status in fork choice (PENDING → FULL).

## Context

Gloas introduces Enshrined Proposer-Builder Separation (ePBS), which fundamentally changes how blocks are processed in fork choice:

1. **Two-phase block structure**:
   - Proposer creates and broadcasts a "beacon block" containing a selected `ExecutionBid`
   - Builder (who made the winning bid) reveals the actual `ExecutionPayload` separately
   
2. **New message types to handle in fork choice**:
   - `ExecutionBid` - builder's bid for a slot
   - `PayloadAttestation` - PTC members attest to payload availability
   
3. **Payload withholding scenario**:
   - Builder might not reveal payload after winning the bid
   - Chain must still progress even if payload is withheld
   - Need to handle orphaning of blocks with missing payloads

## Reference

- **Spec**: https://github.com/ethereum/consensus-specs/blob/dev/specs/gloas/fork-choice.md
- **ePBS EIP**: https://eips.ethereum.org/EIPS/eip-7732

## Implementation Plan

### Task 1: Track payload reveal status in fork choice ✅

Implemented a minimal fork-choice compatibility layer for ePBS:
- New `PayloadStatus` constants in `types::consts::gloas`
- Proto-array stores `payload_status` per block node
- Gloas blocks start as `PENDING`, pre-gloas blocks are `FULL`
- New fork-choice hook `on_execution_payload(block_root)` marks a node `FULL` when the payload envelope arrives

This is the smallest useful step towards proper Gloas fork choice.

### Task 2: Add new message types to fork choice handlers

Files to modify:
- `consensus/fork_choice/src/fork_choice.rs` - main fork choice logic
- `beacon_node/beacon_chain/src/beacon_chain.rs` - integration layer

New functions needed:
- `on_execution_bid()` - handle incoming bids from builders
- `on_payload_attestation()` - handle PTC attestations
- Modify `on_block()` - handle proposer blocks that reference bids instead of containing payloads directly

### Task 2: Update fork choice store

Files to modify:
- `consensus/fork_choice/src/fork_choice_store.rs`

New state to track:
- Available execution bids per slot
- Payload attestations received
- Payload availability status (has quorum been reached?)

### Task 3: Proto-array changes

Files to modify:
- `consensus/proto_array/src/proto_array.rs`
- `consensus/proto_array/src/proto_array_fork_choice.rs`

Changes needed:
- Block nodes need to track whether payload has been revealed
- Add payload availability as a validity condition
- Handle blocks with missing payloads in best chain calculation

### Task 4: Equivocation detection

Files to modify:
- `consensus/fork_choice/src/fork_choice.rs`

New equivocation types:
- Builder submitting multiple bids for same slot
- Builder equivocating on payload reveal

### Task 5: Fork boundary handling

Test fork choice across the fulu -> gloas transition:
- Before gloas: blocks contain full payloads
- After gloas: blocks reference bids, payloads come separately

### Task 6: Testing

- Unit tests for each new function
- Integration tests for two-phase block flow
- Edge case tests (missing payload, late payload, conflicting bids)
- Fork transition tests

## Current Status

**2026-02-14 07:04**: Starting research phase. Reading gloas beacon-chain spec.

**Key findings from spec**:
1. Fork choice is **NOT separately documented** for gloas — it appears to be integrated into beacon-chain.md
2. Two-phase block processing:
   - Beacon block contains `SignedExecutionPayloadBid` (not the full payload)
   - Execution payload comes separately in `SignedExecutionPayloadEnvelope`
   - State transition is split: `state_transition(state, signed_block)` then `process_execution_payload(state, signed_envelope)`
3. New message types:
   - `PayloadAttestation` - PTC attestations with `payload_present` + `blob_data_available` flags
   - `ExecutionPayloadBid` - builder's bid (includes `block_hash`, `value`, `blob_kzg_commitments`)
   - `ExecutionPayloadEnvelope` - actual payload reveal

**State tracking**:
- `state.latest_execution_payload_bid` - the bid accepted in the current block
- `state.execution_payload_availability` - bitvector tracking which slots had payloads revealed
- `state.builder_pending_payments` - payments awaiting PTC quorum
- `state.latest_block_hash` - tracks revealed payload block hashes

## Next Steps

1. ✅ Read gloas beacon-chain spec
2. Search for gloas-specific fork choice modifications (may be in separate PR/doc)
3. Design fork choice message handlers based on beacon-chain state transitions
4. Implement `on_execution_bid` and `on_payload_attestation` handlers
5. Wire up to beacon chain
6. Add tests

## Open Questions

- How do we handle reorgs when payload hasn't been revealed yet?
- What's the timeout for payload revelation?
- Do we need a separate gossip topic subscription mechanism?
- How does this interact with optimistic sync?

## Blockers

None currently.

## Resources

- Existing `on_block` implementation as reference: `consensus/fork_choice/src/fork_choice.rs:659`
- State transition gloas code: `consensus/state_processing/src/per_block_processing/gloas.rs`
