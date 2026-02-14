# Gloas ePBS Equivocation Detection

## Overview

ePBS introduces new message types that validators and builders can equivocate on:
1. **Builder bid equivocation**: Builder submits multiple different bids for the same slot
2. **Payload attestation equivocation**: PTC member submits conflicting attestations for the same slot/block

## Detection Strategy

### 1. Builder Bid Equivocation

**What it is:**
- A builder submits two `SignedExecutionPayloadBid` messages for the same slot with different content
- Example: Bid A with value=10 ETH, Bid B with value=5 ETH, both signed by same builder for slot N

**Where to detect:**
- **Gossip validation** (Phase 4): When processing `execution_bid` gossip messages
- Track seen bids per builder per slot in a cache
- If second bid with different root arrives, mark builder as equivocating

**Consequences:**
- Builder should be slashed (lose their stake)
- Both conflicting bids are rejected
- Builder marked in `fc_store.equivocating_builders()` (needs new tracking structure)

**Implementation location:**
- `beacon_node/network/src/beacon_processor/worker/gossip_methods.rs`
- New function: `verify_execution_bid_for_gossip()`
- Add `equivocating_builders: BTreeSet<BuilderIndex>` to `ForkChoiceStore` trait

### 2. Payload Attestation Equivocation

**What it is:**
- A PTC member submits two conflicting `PayloadAttestation` messages
- Conflict means: same (validator_index, slot, beacon_block_root) but different `payload_present` value
- Example: Validator 42 attests "payload present" then later attests "payload NOT present" for same block

**Where to detect:**
- **Gossip validation** (Phase 4): When processing `payload_attestation` gossip messages
- Track seen attestations per validator per slot+block
- If conflicting attestation arrives, mark validator as equivocating

**Consequences:**
- Validator should be slashed (via attester slashing mechanism)
- Both conflicting attestations rejected
- Validator added to existing `fc_store.equivocating_indices()`

**Implementation location:**
- `beacon_node/network/src/beacon_processor/worker/gossip_methods.rs`
- New function: `verify_payload_attestation_for_gossip()`
- Reuse existing `equivocating_indices` from attestation slashing

## Phase 3 vs Phase 4 Separation

**Phase 3 (Fork Choice)**: ✅ COMPLETE
- Fork choice layer assumes equivocations are already detected
- Uses `fc_store.equivocating_indices()` to filter invalid attestations
- Uses `fc_store.equivocating_builders()` (to be added) to filter invalid bids

**Phase 4 (P2P Networking)**: 🚧 NOT YET STARTED
- P2P gossip validation is where equivocations are actively detected
- Seen message caches track what each builder/validator has sent
- On conflict detection, mark as equivocating and reject future messages

## Caching Strategy

### Builder Bid Cache
```rust
struct SeenBids {
    // Map: (builder_index, slot) -> bid_root
    seen: HashMap<(BuilderIndex, Slot), Hash256>,
}
```

On gossip receipt:
1. Hash the bid
2. Check if we've seen this (builder, slot) pair
3. If yes and hash differs: **EQUIVOCATION** → reject + slash
4. If yes and hash matches: duplicate, ignore
5. If no: store and continue validation

### Payload Attestation Cache
```rust
struct SeenPayloadAttestations {
    // Map: (validator_index, slot, beacon_block_root) -> payload_present
    seen: HashMap<(u64, Slot, Hash256), bool>,
}
```

On gossip receipt:
1. Extract (validator, slot, block_root, payload_present)
2. Check if we've seen this (validator, slot, block_root) triple
3. If yes and payload_present differs: **EQUIVOCATION** → reject + slash
4. If yes and payload_present matches: duplicate, ignore
5. If no: store and continue validation

## Slashing Mechanisms

### Builder Slashing (New)
- Requires new slashing operation type: `BuilderBidEquivocation`
- Contains: two conflicting `SignedExecutionPayloadBid` messages
- Penalty: full builder stake forfeit
- Implementation: Phase 5+ (beacon chain operations)

### Validator Slashing (Existing)
- Reuse attester slashing mechanism
- Create `AttesterSlashing` with two conflicting payload attestations
- Penalty: standard attester slashing penalty
- Implementation: already exists, just wire up payload attestations

## Fork Choice Integration

### Current State (Phase 3)
The fork choice handlers (`on_execution_bid`, `on_payload_attestation`) assume inputs are valid.

### Required Changes (Phase 4)
When equivocation tracking is added to `ForkChoiceStore`:

1. **Add builder equivocation tracking:**
```rust
pub trait ForkChoiceStore<E: EthSpec>: Sized {
    // ... existing methods ...
    
    /// Gets the equivocating builder indices.
    fn equivocating_builders(&self) -> &BTreeSet<BuilderIndex>;
    
    /// Adds to the set of equivocating builders.
    fn extend_equivocating_builders(&mut self, builders: impl IntoIterator<Item = BuilderIndex>);
}
```

2. **Update `on_execution_bid` to reject equivocating builders:**
```rust
pub fn on_execution_bid(&mut self, bid: &SignedExecutionPayloadBid<E>, ...) -> Result<...> {
    // Check if builder has been marked as equivocating
    if self.fc_store.equivocating_builders().contains(&bid.message.builder_index) {
        return Err(InvalidExecutionBid::EquivocatingBuilder { 
            builder_index: bid.message.builder_index 
        }.into());
    }
    
    // ... rest of validation ...
}
```

3. **Update `on_payload_attestation` to filter equivocating validators:**
```rust
pub fn on_payload_attestation(&mut self, attestation: &PayloadAttestation<E>, ...) -> Result<...> {
    // Filter out equivocating validators from indexed attestation
    let non_equivocating_indices: Vec<u64> = indexed_attestation
        .attesting_indices
        .iter()
        .filter(|idx| !self.fc_store.equivocating_indices().contains(*idx))
        .copied()
        .collect();
    
    let attester_count = non_equivocating_indices.len() as u64;
    
    // ... use filtered count for weight accumulation ...
}
```

## Testing Strategy

### Unit Tests (Phase 4)
- `test_builder_bid_equivocation_detection()`
- `test_payload_attestation_equivocation_detection()`
- `test_equivocating_builder_bid_rejected()`
- `test_equivocating_validator_weight_excluded()`

### Integration Tests (Phase 4)
- Simulate builder submitting two different bids
- Verify second bid rejected + builder slashed
- Simulate PTC member flipping payload_present vote
- Verify second attestation rejected + validator slashed

### Spec Tests (Phase 5+)
- Consensus-spec-tests will include equivocation vectors when gloas is finalized
- Run all `operations/builder_bid_equivocation` tests
- Run all `operations/payload_attestation_equivocation` tests

## Implementation Phases

### Phase 3 (Current): ✅ Foundation
- Fork choice handlers written
- Equivocating indices already tracked for attestations
- No active detection (relies on external input)

### Phase 4 (Next): 🚧 Gossip Validation
- Implement seen bid/attestation caches
- Add equivocation detection logic
- Reject and mark equivocators
- Add `equivocating_builders` to ForkChoiceStore

### Phase 5 (Later): 🚧 Slashing Operations
- Add BuilderBidEquivocation operation type
- Wire into block processing
- Test slashing penalties

## Spec References

- ePBS EIP-7732: https://eips.ethereum.org/EIPS/eip-7732
- Gloas fork choice: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/fork-choice.md
- Attester slashing: https://github.com/ethereum/consensus-specs/blob/master/specs/phase0/beacon-chain.md#attesterslashing

## Status

- **Documentation**: ✅ Complete
- **Phase 3 (Fork Choice)**: ✅ Complete (handlers ready, awaiting detection)
- **Phase 4 (P2P Detection)**: ❌ Not started (blocked on gossip implementation)
- **Phase 5 (Slashing Ops)**: ❌ Not started

**Next action**: Proceed to Phase 4 P2P Networking implementation. Equivocation detection will be implemented during gossip validation.

---

*Document created: 2026-02-14 09:45*
*ethvibes 🎵*
