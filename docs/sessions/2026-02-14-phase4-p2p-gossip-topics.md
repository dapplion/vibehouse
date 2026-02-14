# 2026-02-14 09:45 - Phase 4: P2P Networking - Gossip Topics

## ethvibes session: gloas 24/7 🎵

### Mission
Implement Phase 4 of gloas: P2P Networking for ePBS

### Context
- **Phase 1 (Types)**: ✅ COMPLETE
- **Phase 2 (State Transitions)**: ✅ COMPLETE
- **Phase 3 (Fork Choice)**: ✅ COMPLETE
- **Phase 4 (P2P Networking)**: 🚧 STARTING NOW

### Work Plan
Phase 4 breaks down into:
1. Add gossip topics (execution_bid, execution_payload, payload_attestation)
2. Implement gossip validation for each topic
3. Wire up beacon processor handlers
4. Add equivocation detection caches
5. Update peer scoring
6. Tests

### Session: Add Gossip Topics

#### What Was Done

**File**: `beacon_node/lighthouse_network/src/types/topics.rs`

1. **Added topic name constants:**
```rust
pub const EXECUTION_BID_TOPIC: &str = "execution_bid";
pub const EXECUTION_PAYLOAD_TOPIC: &str = "execution_payload";
pub const PAYLOAD_ATTESTATION_TOPIC: &str = "payload_attestation";
```

2. **Extended GossipKind enum:**
```rust
pub enum GossipKind {
    // ... existing variants ...
    
    /// Gloas ePBS: Topic for builders to publish execution payload bids.
    ExecutionBid,
    /// Gloas ePBS: Topic for builders to reveal execution payload envelopes.
    ExecutionPayload,
    /// Gloas ePBS: Topic for PTC members to publish payload attestations.
    PayloadAttestation,
}
```

3. **Updated core_topics_to_subscribe:**
```rust
if fork_name.gloas_enabled() {
    // Gloas ePBS topics - all nodes subscribe to these
    topics.push(GossipKind::ExecutionBid);
    topics.push(GossipKind::ExecutionPayload);
    topics.push(GossipKind::PayloadAttestation);
}
```

4. **Marked new topics as core-only:**
Updated `is_fork_non_core_topic()` to classify them with other core topics

5. **Added decode support:**
Extended topic decode to parse the new topic strings

6. **Added Display support:**
Extended Display impl to format new topics correctly

#### Topic Format

The new topics follow existing conventions:
- `/eth2/{fork_digest}/execution_bid/ssz_snappy`
- `/eth2/{fork_digest}/execution_payload/ssz_snappy`
- `/eth2/{fork_digest}/payload_attestation/ssz_snappy`

All use SSZ+Snappy encoding like existing topics.

#### Auto-Subscription

When the gloas fork activates:
- All beacon nodes automatically subscribe to the 3 new topics
- No manual subscription needed (core topics)
- Fork digest changes → topic strings change → peers re-subscribe

#### What's Next

1. **Gossip validation** - implement validation functions for each topic
2. **Beacon processor** - wire up handlers to process validated messages
3. **Equivocation detection** - add caches to detect conflicting messages
4. **Peer scoring** - configure scores for new topics
5. **Tests** - unit + integration tests

#### Commit
- `p2p: add gloas ePBS gossip topics (execution_bid, execution_payload, payload_attestation)`

### Status

**Phase 4 Progress**: 1/6 items complete
- ✅ Gossip topics added
- 🚧 Gossip validation (next)
- ⏸️ Beacon processor integration
- ⏸️ Equivocation detection
- ⏸️ Peer scoring
- ⏸️ Tests

**Vibes**: Strong. Topics infrastructure is clean and follows existing patterns. Ready to implement validation logic.

🎵 ethvibes
