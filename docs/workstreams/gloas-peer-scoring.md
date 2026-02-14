# Gloas ePBS Peer Scoring Configuration

**Status**: Ready to implement
**Date**: 2026-02-14
**Phase**: 4 Step 4 (P2P Networking)

## Overview

Add peer scoring parameters for the 3 new gloas ePBS gossip topics to prevent spam and penalize bad actors.

## Topics to Score

1. **ExecutionBid** - Builder bid submissions (1 per slot per builder)
2. **ExecutionPayload** - Builder payload reveals (1 per slot)
3. **PayloadAttestation** - PTC attestations (512 validators × ~60% participation)

## Scoring Strategy

### ExecutionBid

**Expected behavior**:
- 1 valid bid per slot per active builder
- Typical: 5-20 builders active
- Peak: 100+ builders possible

**Weight**: Similar to `BeaconBlock` (0.5) - critical consensus message

**Parameters**:
```rust
const EXECUTION_BID_WEIGHT: f64 = 0.5;

params.topics.insert(
    get_hash(GossipKind::ExecutionBid),
    Self::get_topic_params(
        self,
        EXECUTION_BID_WEIGHT,
        1.0,                    // Expected rate: ~1 per slot (winning bid)
        self.epoch * 20,        // Message retention: 20 epochs
        Some((
            E::slots_per_epoch() * 5,  // Burst window: 5 epochs
            3.0,                        // Max burst multiplier
            self.epoch,                 // Decay window
            current_slot
        )),
    ),
);
```

**Rationale**:
- High weight because invalid bids can DoS the proposal process
- Expected rate = 1 (only winning bid propagates widely)
- Long retention to catch delayed equivocations

### ExecutionPayload

**Expected behavior**:
- 1 payload reveal per slot (from winning builder)
- Only after proposer selects bid

**Weight**: Similar to `BeaconBlock` (0.5) - critical for block completion

**Parameters**:
```rust
const EXECUTION_PAYLOAD_WEIGHT: f64 = 0.5;

params.topics.insert(
    get_hash(GossipKind::ExecutionPayload),
    Self::get_topic_params(
        self,
        EXECUTION_PAYLOAD_WEIGHT,
        1.0,                    // Expected rate: 1 per slot
        self.epoch * 20,        // Message retention: 20 epochs
        Some((
            E::slots_per_epoch() * 5,
            3.0,
            self.epoch,
            current_slot
        )),
    ),
);
```

**Rationale**:
- Withholding payload is slashable → high weight justified
- Expected rate = 1 (single winning builder)

### PayloadAttestation

**Expected behavior**:
- 512 PTC members per slot
- ~60% participation = ~307 attestations per slot
- Aggregated (not individual) attestations

**Weight**: Similar to `BeaconAggregateAndProof` (0.5)

**Parameters**:
```rust
const PAYLOAD_ATTESTATION_WEIGHT: f64 = 0.4;  // Slightly lower than block/bid

params.topics.insert(
    get_hash(GossipKind::PayloadAttestation),
    Self::get_topic_params(
        self,
        PAYLOAD_ATTESTATION_WEIGHT,
        PTC_SIZE as f64 * 0.6,  // Expected: 307 attestations per slot
        self.epoch * 4,          // Message retention: 4 epochs
        Some((
            E::slots_per_epoch() * 2,
            2.0,                     // Lower burst tolerance
            self.epoch / 2,
            current_slot
        )),
    ),
);
```

**Rationale**:
- Many messages per slot (unlike block/bid)
- Lower weight to avoid over-penalizing attestation storms
- Shorter retention (attestations matter less after finality)

## Updated Weight Constants

Add to top of `gossipsub_scoring_parameters.rs`:

```rust
const EXECUTION_BID_WEIGHT: f64 = 0.5;
const EXECUTION_PAYLOAD_WEIGHT: f64 = 0.5;
const PAYLOAD_ATTESTATION_WEIGHT: f64 = 0.4;
```

## Updated Max Positive Score

Update calculation in `PeerScoreSettings::new()`:

```rust
let max_positive_score = (MAX_IN_MESH_SCORE + MAX_FIRST_MESSAGE_DELIVERIES_SCORE)
    * (BEACON_BLOCK_WEIGHT
        + BEACON_AGGREGATE_PROOF_WEIGHT
        + beacon_attestation_subnet_weight * chain_spec.attestation_subnet_count as f64
        + VOLUNTARY_EXIT_WEIGHT
        + PROPOSER_SLASHING_WEIGHT
        + ATTESTER_SLASHING_WEIGHT
        + EXECUTION_BID_WEIGHT           // NEW
        + EXECUTION_PAYLOAD_WEIGHT       // NEW
        + PAYLOAD_ATTESTATION_WEIGHT     // NEW
    );
```

## Integration Point

**File**: `beacon_node/lighthouse_network/src/service/gossipsub_scoring_parameters.rs`

**Location**: In `get_peer_score_params()`, after fixed topics (voluntary exit, slashings), before dynamic topics:

```rust
// ... existing fixed topics ...

// Gloas ePBS topics
if enr_fork_id.fork_name.gloas_enabled() {
    params.topics.insert(
        get_hash(GossipKind::ExecutionBid),
        Self::get_topic_params(/* ... */),
    );
    
    params.topics.insert(
        get_hash(GossipKind::ExecutionPayload),
        Self::get_topic_params(/* ... */),
    );
    
    params.topics.insert(
        get_hash(GossipKind::PayloadAttestation),
        Self::get_topic_params(/* ... */),
    );
}

// ... dynamic topics ...
```

## Penalty Behavior

### Invalid Message Penalties

When gossip validation rejects a message:

1. **Invalid signature** → `graylist_threshold` (-16000) immediate penalty
2. **Equivocation** → permanent mark + `graylist_threshold`
3. **Future slot** → small penalty (-100), retry later
4. **Unknown parent** → no penalty, queue for reprocessing

### Mesh Quality Penalties

Peers that:
- Send late messages (> 2s after slot start) → slow peer penalty
- Send duplicate bids → gossip threshold penalty
- Never send valid first messages → mesh message deliveries penalty

## Testing

### Test Scenarios

1. **Normal operation**: Peer sends 1 bid/slot → positive score accumulation
2. **Builder equivocation**: Peer sends 2 different bids for same slot → graylist
3. **Spam attack**: Peer sends 100 bids/slot → rapid score decay
4. **Late delivery**: Peer consistently sends attestations >2s late → slow peer penalty
5. **Fork transition**: Score parameters update when gloas activates

### Assertions

- Normal peer maintains score > `gossip_threshold` (-4000)
- Equivocating peer drops below `graylist_threshold` (-16000)
- Spammy peer gets `IGNORE` on subsequent messages

## Open Questions

1. **Aggregation**: Should we aggregate payload attestations before gossip?
   - Current plan: No aggregation (512 separate messages)
   - Alternative: Aggregate per subnet/committee
   - Decision: Defer to Phase 6 (after initial testing)

2. **Rate limiting**: Should builders be rate-limited per epoch?
   - Each builder can submit 1 bid per slot
   - With 100 builders, that's 3200 bids/epoch
   - Current gossip limits may need adjustment
   - Monitor in devnet, adjust if needed

3. **Payload attestation volume**: Is 307 messages/slot sustainable?
   - Compare to beacon attestations: 64 subnets × avg 32 validators = ~2048/slot
   - PTC is much smaller (307/slot vs 2048/slot)
   - Should be fine, but monitor bandwidth

## Implementation Checklist

- [ ] Add 3 weight constants
- [ ] Update `max_positive_score` calculation
- [ ] Add ExecutionBid scoring params
- [ ] Add ExecutionPayload scoring params
- [ ] Add PayloadAttestation scoring params
- [ ] Guard with `gloas_enabled()` check
- [ ] Update thresholds if needed
- [ ] Write unit tests for score calculation
- [ ] Integration test: verify equivocation → graylist

## Estimated Effort

- **Weight constants**: 5 min
- **Scoring params**: 30 min
- **Integration**: 15 min
- **Testing**: 1 hour

**Total**: ~2 hours

---

**Status**: Design complete. Ready to implement when compilation is possible.
