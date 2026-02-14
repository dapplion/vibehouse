# Gloas Phase 3-8 Implementation Roadmap

## Overview
Phases 1 & 2 (Types & State Transitions) are COMPLETE. This document provides detailed implementation guidance for Phases 3-8.

---

## Phase 3: Fork Choice (High Priority)

### Context
ePBS fundamentally changes fork choice because blocks are now two-phase:
1. **Proposer block**: Contains builder bid commitment
2. **Builder payload**: Revealed separately by builder

Fork choice must track both and handle cases where payload is withheld.

### Key Files
- `consensus/fork_choice/src/fork_choice.rs`
- `consensus/proto_array/src/proto_array.rs`
- `consensus/proto_array/src/proto_array_fork_choice.rs`

### Implementation Steps

#### 3.1 Update ProtoArray Node
**File**: `consensus/proto_array/src/proto_array.rs`

Add fields to `ProtoNode`:
```rust
pub struct ProtoNode {
    // ... existing fields ...
    
    // Gloas ePBS fields
    pub builder_index: Option<BuilderIndex>,           // Which builder bid was chosen
    pub payload_revealed: bool,                         // Has builder revealed payload?
    pub ptc_weight: u64,                                // PTC attestation weight
}
```

#### 3.2 Implement `on_execution_bid`
**File**: `consensus/fork_choice/src/fork_choice.rs`

```rust
/// Process a builder's execution payload bid.
///
/// This is called when a builder submits their bid for a slot.
/// The bid is stored but the payload is not yet revealed.
pub fn on_execution_bid(
    &mut self,
    bid: &SignedExecutionPayloadBid<E>,
    bid_root: Hash256,
    spec: &ChainSpec,
) -> Result<(), Error<E::Error>> {
    // 1. Validate bid signature
    // 2. Check builder exists and is active
    // 3. Verify bid value <= builder balance
    // 4. Store bid in ProtoArray node
    // 5. Mark payload as NOT yet revealed
}
```

**Spec reference**: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/fork-choice.md#on_execution_bid

#### 3.3 Implement `on_payload_attestation`
**File**: `consensus/fork_choice/src/fork_choice.rs`

```rust
/// Process a PTC payload attestation.
///
/// When enough attestations arrive (quorum), the payload is considered
/// available and the block becomes viable for head selection.
pub fn on_payload_attestation(
    &mut self,
    attestation: &PayloadAttestation<E>,
    spec: &ChainSpec,
) -> Result<(), Error<E::Error>> {
    // 1. Validate attestation (slot, beacon_block_root)
    // 2. Get the block this attestation is for
    // 3. Accumulate PTC weight
    // 4. If quorum reached:
    //    - Mark payload_revealed = true (if payload_present)
    //    - Trigger builder payment processing
}
```

**Spec reference**: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/fork-choice.md#on_payload_attestation

#### 3.4 Update `get_head` Logic
**File**: `consensus/fork_choice/src/fork_choice.rs`

Modify head selection to prefer blocks with revealed payloads:
- Blocks without revealed payloads are NOT eligible for head
- If no blocks with revealed payloads exist, may need to consider withholding penalty

**Weight calculation changes**:
- PTC attestations contribute to block weight
- Builder bids affect block viability but not weight directly

#### 3.5 Handle Payload Withholding
If a builder doesn't reveal their payload after winning the bid:
- Block remains in fork choice tree but ineligible for head
- After a timeout (N slots), impose penalty or allow proposer fallback
- Slash builder if they signed bid but didn't reveal?

**Decision needed**: Consult spec for exact withholding penalty mechanism.

#### 3.6 Tests
- Unit tests for `on_execution_bid`
- Unit tests for `on_payload_attestation`
- Integration test: proposer → bid → PTC → head update
- Edge case: multiple competing bids for same slot
- Edge case: payload never revealed (withholding)

---

## Phase 4: P2P Networking (High Priority)

### Context
Gloas introduces new gossipsub topics for ePBS messages.

### Key Files
- `beacon_node/network/src/service/mod.rs`
- `beacon_node/network/src/types/topics.rs`
- `beacon_node/eth2_libp2p/src/types/topics.rs`
- `beacon_node/network/src/beacon_processor/worker/gossip_methods.rs`

### Implementation Steps

#### 4.1 Add New Gossip Topics
**File**: `beacon_node/eth2_libp2p/src/types/topics.rs`

```rust
pub enum GossipKind {
    // ... existing variants ...
    
    // Gloas topics
    ExecutionBid,                    // Builder submits bid
    ExecutionPayload,                // Builder reveals payload
    PayloadAttestation,              // PTC members attest
}
```

For each topic, define:
- Topic string: `/eth2/{fork_digest}/execution_bid/ssz_snappy`
- Encoding: SSZ with snappy compression
- Message type: `SignedExecutionPayloadBid`, `SignedExecutionPayloadEnvelope`, `PayloadAttestation`

#### 4.2 Implement Gossip Validation
**File**: `beacon_node/network/src/beacon_processor/worker/gossip_methods.rs`

For each new topic, implement validation functions:

**`verify_execution_bid_for_gossip`**:
- Check bid slot is current or recent (not too old)
- Verify builder signature
- Check builder exists and is active
- Verify bid value <= builder balance
- Check parent_block_root is known

**`verify_execution_payload_for_gossip`**:
- Check payload matches a known bid
- Verify payload structure and hashes
- Check blob sidecar availability if needed
- Verify builder signature on envelope

**`verify_payload_attestation_for_gossip`**:
- Check attestation slot is current
- Verify aggregation bitfield validity
- Check beacon_block_root is known
- Verify aggregate BLS signature
- Check attesters are in PTC for this slot

#### 4.3 Beacon Processor Integration
**File**: `beacon_node/network/src/beacon_processor/worker/mod.rs`

Add worker functions to process validated messages:
- `process_gossip_execution_bid`
- `process_gossip_execution_payload`
- `process_gossip_payload_attestation`

Each function should:
1. Call fork choice handler (`on_execution_bid`, etc.)
2. Update local caches
3. Propagate to peers (if valid)

#### 4.4 Topic Subscription Management
**File**: `beacon_node/network/src/service/mod.rs`

At Gloas fork activation:
- Subscribe to new topics: `execution_bid`, `execution_payload`, `payload_attestation`
- Update topic filters and peer scoring

At Gloas fork boundary:
- Handle transition from Fulu topics to Gloas topics
- Maintain overlap during fork transition window

#### 4.5 Update Peer Scoring
**File**: `beacon_node/eth2_libp2p/src/behaviour/gossipsub_scoring_parameters.rs`

Add scoring parameters for new topics:
- Weight per topic
- Penalties for invalid messages
- Rewards for useful messages

#### 4.6 Tests
- Gossip validation unit tests for each new topic
- Integration test: publish message, verify propagation
- Test fork boundary: subscription changes
- Test peer scoring: invalid message penalties

---

## Phase 5: Beacon Chain Integration (High Priority)

### Context
Wire up fork choice and P2P into the beacon chain event loop.

### Key Files
- `beacon_node/beacon_chain/src/beacon_chain.rs`
- `beacon_node/beacon_chain/src/block_verification.rs`
- `beacon_node/beacon_chain/src/builder_cache.rs` (new)

### Implementation Steps

#### 5.1 Create Builder Cache
**File**: `beacon_node/beacon_chain/src/builder_cache.rs` (new)

```rust
/// In-memory cache of builder bids for recent slots.
pub struct BuilderCache<E: EthSpec> {
    /// Bids indexed by slot
    bids: LruCache<Slot, HashMap<BuilderIndex, SignedExecutionPayloadBid<E>>>,
    
    /// Revealed payloads indexed by (slot, builder_index)
    payloads: LruCache<(Slot, BuilderIndex), SignedExecutionPayloadEnvelope<E>>,
}

impl<E: EthSpec> BuilderCache<E> {
    pub fn insert_bid(&mut self, bid: SignedExecutionPayloadBid<E>);
    pub fn get_bids(&self, slot: Slot) -> Vec<&SignedExecutionPayloadBid<E>>;
    pub fn get_best_bid(&self, slot: Slot) -> Option<&SignedExecutionPayloadBid<E>>;
    pub fn insert_payload(&mut self, payload: SignedExecutionPayloadEnvelope<E>);
    pub fn get_payload(&self, slot: Slot, builder_index: BuilderIndex) -> Option<&SignedExecutionPayloadEnvelope<E>>;
}
```

#### 5.2 Update Block Import Pipeline
**File**: `beacon_node/beacon_chain/src/block_verification.rs`

Modify block verification to handle two-phase blocks:

**For proposer blocks**:
- Verify execution_payload_bid (if present)
- Store bid in BuilderCache
- Call `fork_choice.on_execution_bid()`
- Mark block as "awaiting payload"

**For builder payloads**:
- Lookup corresponding proposer block
- Verify payload matches bid commitment
- Store payload in BuilderCache
- Update state with full block

**Key challenge**: Block import is currently atomic. With ePBS, it becomes two-phase:
1. Import proposer block (incomplete)
2. Wait for builder payload
3. Complete block import

**Solution**: Introduce `PartialBlock` state in fork choice?

#### 5.3 Update Chain Head Tracking
**File**: `beacon_node/beacon_chain/src/beacon_chain.rs`

Modify `head` tracking to account for:
- Blocks with bids but no revealed payload → not viable for head
- PTC quorum achievement → payload becomes available, head may update

Add method:
```rust
pub fn on_payload_revealed(&self, slot: Slot, builder_index: BuilderIndex) -> Result<(), Error> {
    // 1. Get proposer block for this slot
    // 2. Verify payload matches bid
    // 3. Update fork choice: payload now available
    // 4. Recompute head (may change)
    // 5. Trigger attestation production if head changed
}
```

#### 5.4 Implement Payload Timeliness Logic
**File**: `beacon_node/beacon_chain/src/ptc.rs` (new)

```rust
/// Manages PTC (Payload Timeliness Committee) logic.
pub struct PtcManager<E: EthSpec> {
    /// Cache of PTC committees per slot
    committees: LruCache<Slot, Vec<u64>>,
    
    /// Aggregated attestations per slot
    attestations: HashMap<Slot, Vec<PayloadAttestation<E>>>,
}

impl<E: EthSpec> PtcManager<E> {
    /// Get PTC committee for a slot (compute or retrieve from cache)
    pub fn get_committee(&mut self, state: &BeaconState<E>, slot: Slot, spec: &ChainSpec) -> Result<Vec<u64>, Error>;
    
    /// Add attestation to aggregate
    pub fn add_attestation(&mut self, attestation: PayloadAttestation<E>) -> Result<(), Error>;
    
    /// Check if quorum reached for a slot
    pub fn check_quorum(&self, slot: Slot, spec: &ChainSpec) -> bool;
}
```

#### 5.5 Tests
- Unit test: builder cache operations
- Unit test: PTC manager aggregation
- Integration test: full block import with two-phase flow
- Integration test: head update after payload revealed
- Edge case: payload never revealed, block orphaned

---

## Phase 6: Validator Client (Medium Priority)

### Context
Validators need to perform new duties in Gloas:
- **Proposers**: Select builder bid, include in block
- **PTC members**: Attest to payload presence

### Key Files
- `validator_client/src/block_service.rs`
- `validator_client/src/attestation_service.rs`
- `validator_client/src/duties_service.rs`
- `validator_client/src/ptc_service.rs` (new)

### Implementation Steps

#### 6.1 Update Block Proposal Flow
**File**: `validator_client/src/block_service.rs`

Modify `produce_block` to:
1. Query BN for available builder bids at this slot
2. Select best bid (highest value, or self-build if no bids)
3. Include `SignedExecutionPayloadBid` in block
4. Sign and publish proposer block

**API call**:
```rust
// GET /eth/v1/builder/bids/{slot}
// Returns: Vec<SignedExecutionPayloadBid>
let bids = beacon_node_client.get_builder_bids(slot).await?;
let best_bid = select_best_bid(&bids);
```

#### 6.2 Implement Bid Selection Logic
**File**: `validator_client/src/builder_selection.rs` (new)

```rust
/// Select the best builder bid for a slot.
///
/// Strategy:
/// - Prefer highest value bid
/// - Verify builder is active and has sufficient balance
/// - Fallback to self-build if no valid bids
pub fn select_best_bid<E: EthSpec>(
    bids: Vec<SignedExecutionPayloadBid<E>>,
    state: &BeaconState<E>,
    spec: &ChainSpec,
) -> SignedExecutionPayloadBid<E> {
    // Filter valid bids
    // Sort by value descending
    // Return highest, or self-build bid if empty
}
```

#### 6.3 Implement PTC Attestation Duty
**File**: `validator_client/src/ptc_service.rs` (new)

New service to handle PTC duties:
```rust
pub struct PtcService<T: SlotClock + 'static, E: EthSpec> {
    // ...
}

impl<T: SlotClock + 'static, E: EthSpec> PtcService<T, E> {
    /// Check if any validator in our set is in the PTC for this slot
    pub async fn check_ptc_duties(&self, slot: Slot) -> Vec<ValidatorIndex>;
    
    /// Produce and publish payload attestation
    pub async fn produce_payload_attestation(
        &self,
        slot: Slot,
        validator_indices: Vec<u64>,
    ) -> Result<(), Error>;
}
```

**Logic**:
1. At slot start, query BN: "Am I in the PTC for this slot?"
2. If yes, wait for payload to be revealed (or timeout)
3. Check blob availability
4. Produce PayloadAttestation with `payload_present` = true/false
5. Sign with validator key(s)
6. Aggregate signatures if multiple validators in same PTC
7. Publish to gossip

#### 6.4 Update Duty Discovery
**File**: `validator_client/src/duties_service.rs`

Add PTC duty to the duty scheduler:
```rust
pub enum DutyType {
    Attestation,
    BlockProposal,
    SyncCommittee,
    PayloadAttestation,  // New for Gloas
}
```

Query BN at epoch start:
```
GET /eth/v1/validator/duties/ptc/{epoch}
```

#### 6.5 Handle Missing Bids
If no builder bids are received:
- Proposer MUST self-build (fallback)
- Include self-build bid (value = 0, signature = empty)
- Builder client constructs payload locally

#### 6.6 Tests
- Unit test: bid selection logic
- Unit test: PTC attestation production
- Integration test: full block proposal with bid selection
- Integration test: PTC attestation for revealed payload
- Edge case: no bids available → self-build

---

## Phase 7: REST API (Medium Priority)

### Context
Beacon API needs new endpoints for ePBS.

### Key Files
- `beacon_node/http_api/src/lib.rs`
- `beacon_node/http_api/src/builder.rs` (new)
- `beacon_node/http_api/src/ptc.rs` (new)

### Implementation Steps

#### 7.1 Builder Bid Endpoints
**File**: `beacon_node/http_api/src/builder.rs`

```rust
// POST /eth/v1/builder/bids
// Submit a builder bid
pub async fn post_builder_bid(
    bid: SignedExecutionPayloadBid<E>,
    beacon_chain: Arc<BeaconChain<E>>,
) -> Result<(), Error>

// GET /eth/v1/builder/bids/{slot}
// Get all bids for a slot (for proposer to choose from)
pub async fn get_builder_bids(
    slot: Slot,
    beacon_chain: Arc<BeaconChain<E>>,
) -> Result<Vec<SignedExecutionPayloadBid<E>>, Error>

// POST /eth/v1/builder/payload
// Submit revealed execution payload
pub async fn post_builder_payload(
    payload: SignedExecutionPayloadEnvelope<E>,
    beacon_chain: Arc<BeaconChain<E>>,
) -> Result<(), Error>
```

#### 7.2 PTC Endpoints
**File**: `beacon_node/http_api/src/ptc.rs`

```rust
// GET /eth/v1/validator/duties/ptc/{epoch}
// Get PTC duties for validators in an epoch
pub async fn get_ptc_duties(
    epoch: Epoch,
    validator_indices: Vec<u64>,
    beacon_chain: Arc<BeaconChain<E>>,
) -> Result<Vec<PtcDuty>, Error>

// POST /eth/v1/beacon/ptc/attestations
// Submit payload attestation
pub async fn post_payload_attestation(
    attestation: PayloadAttestation<E>,
    beacon_chain: Arc<BeaconChain<E>>,
) -> Result<(), Error>
```

#### 7.3 Update Block Endpoints
Modify existing block endpoints to handle ePBS:
- `GET /eth/v2/beacon/blocks/{block_id}` → include builder bid in response
- `POST /eth/v1/beacon/blinded_blocks` → may need modification for ePBS semantics

#### 7.4 Proposer Lookahead Endpoint
Implement https://github.com/sigp/lighthouse/pull/8815:
```rust
// GET /eth/v1/validator/proposer_duties_lookahead/{epoch}
// Returns proposer duties for future epochs (helps with MEV/builder coordination)
```

#### 7.5 SSE Events
Add Server-Sent Events for new message types:
- `event: execution_bid` → fired when builder bid arrives
- `event: execution_payload` → fired when payload revealed
- `event: payload_attestation` → fired when PTC attestation arrives

#### 7.6 Tests
- API integration tests for each new endpoint
- Test POST → validate → gossip flow
- Test GET → query cache → return data
- Test SSE event firing

---

## Phase 8: Testing & Polish (Ongoing)

### Comprehensive Test Suite

#### 8.1 Spec Tests
- All vectors in `testing/ef_tests/consensus-spec-tests/tests/mainnet/gloas/`
- Target: 100% pass rate

#### 8.2 Integration Tests
- Full block flow: proposer → builder → PTC → finalization
- Fork transition: Fulu → Gloas at epoch boundary
- Multi-client interop: vibehouse + other clients (via Kurtosis)

#### 8.3 Edge Cases
- Builder withholding (doesn't reveal payload)
- Proposer fallback (self-build when no bids)
- PTC quorum not reached
- Conflicting bids for same slot
- Late attestations
- Reorgs with partial ePBS blocks

#### 8.4 Performance Tests
- Throughput: How many bids/attestations per second?
- Latency: Time from bid submission to payload reveal
- Memory: BuilderCache size under load
- Network: Gossip bandwidth with 3 new topics

#### 8.5 Fuzz Testing
Use `cargo-fuzz` to fuzz:
- `process_execution_payload_bid` input
- `process_payload_attestation` input
- SSZ deserialization for new types

---

## Implementation Priority

### Week 1-2: Core Functionality
1. **Phase 3**: Fork choice (blocking for correctness)
2. **Phase 4**: P2P networking (blocking for interop)
3. **Phase 5**: Beacon chain integration (blocking for BN operation)

### Week 3: Validator Support
4. **Phase 6**: Validator client (blocking for staking)

### Week 4: API & Testing
5. **Phase 7**: REST API (important for external integrations)
6. **Phase 8**: Comprehensive testing (ongoing)

### Parallel Workstreams
- **Documentation**: Update `docs/` with ePBS architecture
- **Metrics**: Add Prometheus metrics for builder bids, PTC participation
- **Logging**: Structured logs for debugging ePBS flow
- **Upstream sync**: Cherry-pick relevant commits from Lighthouse #8806

---

## Success Criteria

✅ **Phase 3-8 Complete When**:
- Fork choice handles ePBS correctly
- P2P topics functional and validated
- Beacon chain imports two-phase blocks
- Validator client performs proposer + PTC duties
- REST API serves all ePBS endpoints
- All spec tests pass (fork_choice, networking, full chain)
- Kurtosis multi-client testnet runs successfully with vibehouse

✅ **Ready for Gloas Mainnet When**:
- All phases complete
- Security audit completed
- Performance benchmarks meet targets
- Documentation finalized
- Community testing on devnet successful

---

## Resources

- **Specs**: https://github.com/ethereum/consensus-specs/tree/master/specs/gloas
- **Upstream**: https://github.com/sigp/lighthouse/pull/8806
- **EIP-7732**: https://eips.ethereum.org/EIPS/eip-7732
- **Engine API**: https://github.com/ethereum/execution-apis/tree/main/src/engine

**Next: When toolchain available, validate Phase 1-2, then begin Phase 3.** 🎵
