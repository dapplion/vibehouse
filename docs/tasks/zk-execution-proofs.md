# ZK Execution Proofs for vibehouse

> Task tracking document for implementing zkEVM execution proofs in vibehouse.
> Related: [vibehouse issue #28](https://github.com/AudaciousCapital/vibehouse/issues/28)

## Background and Motivation

### The Problem
Ethereum validators currently need a full Execution Layer (EL) client with ~200GB+ of state to verify blocks. This creates centralization pressure — fewer nodes can afford the hardware. The Ethereum roadmap ("The Verge") aims to fix this with stateless validation: proving execution correctness cryptographically so validators don't need local EL state.

### Why ZK Proofs?
Three approaches to stateless validation exist:
1. **Stateless (with witnesses)**: Block includes state access proofs (Verkle trees). Validators re-execute with proven state. Still requires execution.
2. **Executionless (ZK)**: A zkEVM proof proves the execution was correct. Validators only verify the proof (~milliseconds). No execution, no state.
3. **Hybrid**: Both witnesses and ZK proofs coexist during transition.

ZK proofs are the endgame — they compress an entire block's execution into a ~300KB proof verifiable in milliseconds.

### Why Now?
- vibehouse has **ePBS (EIP-7732/Gloas) fully implemented** — this fundamentally changes the proof architecture
- Kev's prototype (sigp/lighthouse#7755) was built pre-ePBS — it needs rethinking
- The consensus-specs now have EIP-8025 (Optional Execution Proofs) merged (ethereum/consensus-specs#4591)
- zkEVM proving systems (SP1, RISC Zero, Jolt) are approaching real-time proving capability

## How ePBS Changes the Proof Architecture

### Pre-ePBS (Kev's Prototype)
In Kev's PR (#7755), the architecture assumed the standard block flow:
- **Proposer** builds a block (or gets one from a builder via MEV-Boost)
- **Proof generators** are separate nodes that receive the block, re-execute via EL, and generate proofs
- **Stateless attestors** wait for proofs to arrive via gossip subnets before considering the block valid
- Proofs are generated *after* the block is published, racing against the attestation deadline (~4s into the slot)

**Key problem**: In 12-second slots, the proposer publishes at t=0, attestors vote at t=4s. That leaves ~4 seconds for proof generation — far too little for current zkEVM systems (which take 10-60+ seconds).

### With ePBS (vibehouse)
ePBS fundamentally restructures block production:

1. **Proposer** publishes a beacon block containing only a `SignedExecutionPayloadBid` (a commitment to a block hash, no actual payload)
2. **Builder** reveals the actual `ExecutionPayloadEnvelope` later in the slot
3. **Payload attestors** (PTC - Payload Timeliness Committee) attest to payload timeliness

This creates a **natural window for proof generation**:
- The **builder** knows the execution payload *before* the slot (they built it)
- The builder can start proving **before or during** the slot
- The builder has until they reveal the `ExecutionPayloadEnvelope` to attach or separately publish proofs
- Even after reveal, proofs can arrive on subnets before the next slot

**Key insight**: In ePBS, the **builder is the natural proof generator**, not a separate node class. Builders are already well-resourced (GPU farms for MEV extraction) and have the payload earliest.

### Architecture Comparison

| Aspect | Pre-ePBS (Kev's PR) | With ePBS (vibehouse) |
|--------|---------------------|----------------------|
| Who generates proofs | Dedicated "proof generator" nodes | Builders (primarily) |
| When proving starts | After block gossip (t=0 of slot) | Before slot (builder has payload early) |
| Time budget | ~4s (before attestation deadline) | Minutes to full slot (12s+) |
| Proof delivery | Separate gossip subnets | Could bundle with `ExecutionPayloadEnvelope` OR separate subnets |
| Incentive to prove | None (subsidized) | Builder incentive — unproven blocks may get reorged |
| Block structure | Standard beacon block + separate proofs | Beacon block has bid, envelope has payload, proofs accompany envelope |

## Architectural Design

### Core Components

#### 1. Execution Proof Types
Following EIP-8025 and Kev's prototype:

```rust
pub struct ExecutionProof {
    pub block_hash: ExecutionBlockHash,
    pub subnet_id: ExecutionProofSubnetId,
    pub version: u32,
    pub proof_data: Vec<u8>,   // Opaque — depends on proof system
    pub timestamp: u64,
}
```

Multiple proof types map to different subnets (0-7):
- Subnet 0: Execution witness proofs (state proofs)
- Subnet 1: SP1 zkVM proofs
- Subnet 2: RISC Zero proofs
- Subnet 3: Jolt proofs
- etc.

#### 2. Proof Chain (Dual-View Architecture)
From Kev's design — fork choice remains untouched:

- **Optimistic View (Fork Choice)**: All blocks imported optimistically. Fork choice weights NOT modified by proof status. Validators attest normally.
- **Proven View (Proof Store)**: Tracks which blocks have sufficient proofs. Maintains a "proven canonical chain" for monitoring. Independent of fork choice.

This separation is critical: it means we can implement ZK proofs **without touching fork choice logic**, reducing risk dramatically.

#### 3. Integration with ePBS Flow

```
Slot N:
  t=0:  Proposer publishes BeaconBlock (contains SignedExecutionPayloadBid)
        → Attestors attest to beacon block (head vote, no execution validation needed)
  
  t=?:  Builder reveals ExecutionPayloadEnvelope
        → Payload attestors (PTC) check timeliness
        → Builder also publishes ExecutionProof(s) to proof subnets
  
  t=end: Proof(s) arrive on subnets
        → Stateless nodes verify proof, mark block as "proven"
        → Proof store updated, metrics emitted

Slot N+1:
  → Next proposer can see proven status of slot N
  → If slot N unproven, may choose to build on slot N-1 (soft incentive)
```

#### 4. Stateless Attestor Mode
A node running with `--stateless-validation`:
- Does NOT connect to an EL
- Subscribes to execution proof subnets
- Imports all blocks optimistically
- Waits for proofs to mark blocks as "proven" in proof store
- Can still attest (fork choice is optimistic, not proof-dependent)
- **Cannot locally build blocks** — must use ePBS builders (which vibehouse already supports!)

#### 5. Proof Subnets (Gossip Layer)
New gossip topics: `execution_proof_{subnet_id}` (0 through `MAX_EXECUTION_PROOF_SUBNETS - 1`)

Nodes subscribe based on configuration:
- Stateless validators: subscribe to subnets for proof types they accept
- Proof generators: publish to their proof type's subnet
- Regular nodes: optionally subscribe for monitoring

### Integration with vibehouse's ePBS

The key files in vibehouse's ePBS implementation:

| File | Purpose | ZK Proof Relevance |
|------|---------|-------------------|
| `consensus/types/src/execution_payload_envelope.rs` | Builder's payload reveal | Could carry proof commitment or proof hash |
| `consensus/types/src/execution_payload_bid.rs` | Builder's bid | Could signal "will provide ZK proof" |
| `beacon_node/beacon_chain/src/gloas_verification.rs` | ePBS verification logic | Needs proof-aware verification path |
| `beacon_node/beacon_chain/src/execution_bid_pool.rs` | Bid management | May prioritize bids that promise proofs |
| `beacon_node/beacon_chain/src/execution_payload.rs` | Payload processing | Needs stateless bypass |
| `beacon_node/beacon_chain/src/observed_execution_bids.rs` | Bid observation | — |
| `beacon_node/beacon_chain/src/data_availability_checker.rs` | DA checks | Needs execution proof requirement added |
| `beacon_node/beacon_chain/src/chain_config.rs` | Configuration | New stateless/proof flags |
| `beacon_node/beacon_chain/src/builder.rs` | Chain builder | Wire up proof requirements |

## Progress Log

### 2026-02-19 — Task 4: execution proof gossip topics
- **Added `GossipKind::ExecutionProof(ExecutionProofSubnetId)` variant** following the indexed topic pattern (like `DataColumnSidecar(DataColumnSubnetId)`).
- **Topic string**: `execution_proof_{subnet_id}` — parsed via `strip_prefix(EXECUTION_PROOF_PREFIX)` in `subnet_topic_index()`.
- **PubsubMessage::ExecutionProof** variant: `Box<(ExecutionProofSubnetId, Arc<ExecutionProof>)>` — stores subnet ID and proof data. Fork-gated to `gloas_enabled()`. SSZ decode/encode implemented.
- **Router**: Temporary no-op handler with debug log. Full gossip processing (Task 10) comes later.
- **Gossip cache**: `None` (no caching, proofs are time-sensitive like other ePBS messages).
- **Files changed**: 5 modified
  - `beacon_node/lighthouse_network/src/types/topics.rs`: constant, GossipKind variant, Display, decode, subnet_topic_index, is_fork_non_core_topic
  - `beacon_node/lighthouse_network/src/types/pubsub.rs`: imports, PubsubMessage variant, kind(), decode(), encode(), Display
  - `beacon_node/lighthouse_network/src/service/gossip_cache.rs`: ExecutionProof arm
  - `beacon_node/network/src/router.rs`: ExecutionProof routing (debug log placeholder)
- 92/92 lighthouse_network tests pass.

### 2026-02-19 — Phase 1 complete: core types, chain config, CLI flags (Tasks 1-3)
- **Task 1: ExecutionProof and ExecutionProofSubnetId types**
  - Created `consensus/types/src/execution_proof.rs`: `ExecutionProof` struct with `block_root`, `block_hash`, `subnet_id`, `version`, `proof_data` fields. SSZ+serde derives. `is_version_supported()` and `is_structurally_valid()` validation methods. `MAX_EXECUTION_PROOF_SIZE` constant (1MB). 5 unit tests (valid proof, invalid version, empty data, oversized data, SSZ roundtrip).
  - Created `consensus/types/src/execution_proof_subnet_id.rs`: `ExecutionProofSubnetId` newtype wrapper over u64 with bounds checking via `new(id)`. `MAX_EXECUTION_PROOF_SUBNETS` constant (1 for initial rollout). Manual SSZ Encode/Decode impls, serde with quoted_u64. 3 unit tests.
  - Modified `consensus/types/src/lib.rs`: module declarations + `pub use` exports for both types.
  - **Design decisions vs task doc**: Used `block_root: Hash256` instead of `timestamp: u64` (Kev's PR uses block_root, which is more useful for proof-to-block association). Set `MAX_EXECUTION_PROOF_SUBNETS = 1` (start minimal, matching Kev's initial rollout value). Omitted `max_execution_proof_subnets` from ChainConfig since the protocol constant is sufficient.
  - 311/311 types tests pass.
- **Task 2: Chain configuration flags**
  - Added to `ChainConfig`: `stateless_validation: bool` (default false), `generate_execution_proofs: bool` (default false), `stateless_min_proofs_required: usize` (default 1).
  - Omitted `max_execution_proof_subnets` — the protocol constant `MAX_EXECUTION_PROOF_SUBNETS` in the type module is sufficient. If runtime configurability is needed later, it can be added.
  - Full binary compiles.
- **Task 3: CLI flags**
  - Added `--stateless-validation`, `--generate-execution-proofs`, `--stateless-min-proofs-required <N>` flags.
  - `--stateless-min-proofs-required` requires `--stateless-validation` to be set.
  - Wired through `config.rs` to `ChainConfig`.
  - All three flags visible in `lighthouse bn --help`.
- **Files changed**: 5 (2 new, 3 modified)
  - `consensus/types/src/execution_proof.rs` (new)
  - `consensus/types/src/execution_proof_subnet_id.rs` (new)
  - `consensus/types/src/lib.rs` (modified)
  - `beacon_node/beacon_chain/src/chain_config.rs` (modified)
  - `beacon_node/src/cli.rs` (modified)
  - `beacon_node/src/config.rs` (modified)

## Implementation Tasks

### Phase 1: Core Types and Configuration (2-3 sessions)

#### Task 1: Add ExecutionProof and ExecutionProofSubnetId types
**Files to create:**
- `consensus/types/src/execution_proof.rs`
- `consensus/types/src/execution_proof_subnet_id.rs`

**Files to modify:**
- `consensus/types/src/lib.rs` (exports)

**Details:**
- Port `ExecutionProof` struct from Kev's PR (block_hash, subnet_id, version, proof_data, timestamp)
- Port `ExecutionProofSubnetId` with SSZ/serde derives
- Add `MAX_EXECUTION_PROOF_SUBNETS` constant (default 8)
- Add `MAX_EXECUTION_PROOF_SIZE` constant for gossip validation

**Reference:** Kev's `consensus/types/src/execution_proof.rs` (141 lines) and `execution_proof_subnet_id.rs` (133 lines)

---

#### Task 2: Add chain configuration flags
**Files to modify:**
- `beacon_node/beacon_chain/src/chain_config.rs`

**Details:**
Add fields to `ChainConfig`:
```rust
pub stateless_validation: bool,          // default: false
pub generate_execution_proofs: bool,     // default: false  
pub max_execution_proof_subnets: u64,    // default: 8
pub stateless_min_proofs_required: usize, // default: 1
```

**Reference:** Kev's `chain_config.rs` diff (+32 lines)

---

#### Task 3: Add CLI flags
**Files to modify:**
- `beacon_node/src/cli.rs`
- `beacon_node/src/config.rs`

**Details:**
- `--stateless-validation`: Enable stateless mode
- `--generate-execution-proofs`: Enable proof generation
- `--max-execution-proof-subnets <N>`: Subnet count
- `--stateless-min-proofs-required <N>`: Min proofs threshold

**Reference:** Kev's `cli.rs` (+39 lines) and `config.rs` (+50 lines)

---

### Phase 2: Network Layer (2-3 sessions)

#### Task 4: Add execution proof gossip topics
**Files to modify:**
- `beacon_node/lighthouse_network/src/types/topics.rs`
- `beacon_node/lighthouse_network/src/types/pubsub.rs`
- `beacon_node/lighthouse_network/src/types/subnet.rs`

**Details:**
- Add `EXECUTION_PROOF_TOPIC` gossip topic format string
- Add `GossipKind::ExecutionProof(ExecutionProofSubnetId)` variant
- Wire into topic encoding/decoding

**Reference:** Kev's `topics.rs` (+36 lines), `pubsub.rs` (+30 lines), `subnet.rs` (+3 lines)

---

#### Task 5: Add execution proof subnet discovery
**Files to modify:**
- `beacon_node/lighthouse_network/src/discovery/enr.rs`
- `beacon_node/lighthouse_network/src/discovery/mod.rs`
- `beacon_node/lighthouse_network/src/discovery/subnet_predicate.rs`
- `beacon_node/lighthouse_network/src/config.rs`
- `beacon_node/lighthouse_network/src/types/globals.rs`

**Details:**
- Add execution proof subnet bitfield to ENR
- Add subnet predicate for proof subnet discovery
- Update network config for proof subnet count

**Reference:** Kev's ENR changes (+27 lines), discovery (+3 lines)

---

#### Task 6: Gossip subscription and cache
**Files to modify:**
- `beacon_node/lighthouse_network/src/service/utils.rs`
- `beacon_node/lighthouse_network/src/service/gossip_cache.rs`
- `beacon_node/lighthouse_network/src/peer_manager/mod.rs`
- `beacon_node/lighthouse_network/src/peer_manager/peerdb/peer_info.rs`

**Details:**
- Subscribe to execution proof subnets on startup (if stateless_validation or generate_execution_proofs)
- Add proof topics to gossip cache
- Track peer execution proof subnet participation

---

### Phase 3: Proof Verification and Storage (3-4 sessions)

#### Task 7: Proof verification module
**Files to create:**
- `beacon_node/beacon_chain/src/execution_proof_verification.rs`

**Details:**
- `GossipVerifiedExecutionProof<T, O>` wrapper type
- Gossip validation: check block_hash exists, subnet_id valid, version known, size limits
- Stubbed cryptographic verification (return true) — real ZK verification comes later
- `VerifiedExecutionProof` inner type for cache storage

**Reference:** Kev's `execution_proof_verification.rs` (236 lines)

---

#### Task 8: Integrate proofs into DataAvailabilityChecker
**Files to modify:**
- `beacon_node/beacon_chain/src/data_availability_checker.rs`
- `beacon_node/beacon_chain/src/data_availability_checker/overflow_lru_cache.rs`

**Details:**
- Add `ImportableProofData` enum (Proofs(Vec<ExecutionProof>), NoneRequired)
- Add `verified_execution_proofs` HashMap to `PendingComponents`
- Add `min_execution_proofs_required: Option<usize>` to `DataAvailabilityCheckerInner`
- Modify `make_available()` to check execution proof threshold
- Add `put_gossip_verified_execution_proofs()` method
- Add `proof_data` field to `AvailableBlock`

**Reference:** Kev's `data_availability_checker.rs` (+66 lines), `overflow_lru_cache.rs` (+149 lines). This is the largest single change.

---

#### Task 9: Beacon chain proof intake methods  
**Files to modify:**
- `beacon_node/beacon_chain/src/beacon_chain.rs`
- `beacon_node/beacon_chain/src/builder.rs`
- `beacon_node/beacon_chain/src/lib.rs`
- `beacon_node/beacon_chain/src/errors.rs`

**Details:**
- Add `check_gossip_execution_proof_availability_and_import()` to `BeaconChain`
- Wire `min_execution_proofs_required` through builder into DA checker
- Export new modules
- Add proof-related error variants

**Reference:** Kev's `beacon_chain.rs` (+34 lines), `builder.rs` (+8 lines)

---

### Phase 4: Network Processing (2-3 sessions)

#### Task 10: Gossip processing for execution proofs
**Files to modify:**
- `beacon_node/network/src/network_beacon_processor/gossip_methods.rs`
- `beacon_node/network/src/network_beacon_processor/mod.rs`

**Details:**
- Add `process_gossip_execution_proof()` handler
- Validate proof, check against known blocks, store in DA checker
- Add `EXECUTION_PROOF_GOSSIP` work event type to beacon processor

**Reference:** Kev's `gossip_methods.rs` (+171 lines — the second largest change)

---

#### Task 11: Router and service wiring
**Files to modify:**
- `beacon_node/network/src/router.rs`
- `beacon_node/network/src/service.rs`
- `beacon_node/beacon_processor/src/lib.rs`

**Details:**
- Route incoming `GossipKind::ExecutionProof` messages to processor
- Add execution proof work type to beacon processor queue
- Handle proof subnet subscription lifecycle

**Reference:** Kev's `router.rs` (+13 lines), `service.rs` (+11 lines), `beacon_processor` (+13 lines)

---

### Phase 5: ePBS-Specific Integration (2-3 sessions)

#### Task 12: Proof generation trigger on ExecutionPayloadEnvelope
**Files to create:**
- `beacon_node/beacon_chain/src/execution_proof_generation.rs`

**Files to modify:**
- `beacon_node/beacon_chain/src/execution_payload.rs`

**Details:**
- When a node with `generate_execution_proofs` receives an `ExecutionPayloadEnvelope`, trigger proof generation
- Stub out actual proof generation (placeholder that creates a dummy proof)
- In ePBS flow: proof generation starts when builder reveals payload, not at block import
- Modify `notify_new_payload` (or ePBS equivalent) to optionally trigger proof gen

**Reference:** Kev's `execution_proof_generation.rs` (300 lines) — but needs rework for ePBS flow

---

#### Task 13: Proof broadcaster service
**Files to create:**
- `beacon_node/client/src/execution_proof_broadcaster.rs` (or equivalent location)

**Details:**
- Background service that publishes generated proofs to gossip subnets
- Queue-based: proof generation pushes to queue, broadcaster drains and publishes
- Retry logic for failed publications

---

#### Task 14: Modify ePBS verification for stateless mode
**Files to modify:**
- `beacon_node/beacon_chain/src/gloas_verification.rs`
- `beacon_node/beacon_chain/src/execution_payload.rs`

**Details:**
- In stateless mode, skip `engine_newPayload` calls
- Mark execution payloads as optimistic
- The existing `notify_new_payload` path needs a stateless bypass:
  - If `stateless_validation`, return optimistic status instead of calling EL
  - Fork choice updates should use optimistic status

---

#### Task 15: Builder proof commitment (optional, design needed)
**Files to modify:**
- `consensus/types/src/execution_payload_envelope.rs` (potentially)
- `consensus/types/src/execution_payload_bid.rs` (potentially)

**Details:**
- Consider adding optional `proof_commitment` field to `ExecutionPayloadEnvelope`
- Or a `will_provide_proof: bool` flag in `ExecutionPayloadBid`
- This is a spec-level decision — may need discussion with Kev

---

### Phase 6: Events, API, and Observability (1-2 sessions)

#### Task 16: SSE events for proof status
**Files to modify:**
- `beacon_node/beacon_chain/src/events.rs`
- `common/eth2/src/types.rs`

**Details:**
- Add `ExecutionProofReceived` event type
- Add `BlockProvenStatus` event type
- Emit events when proofs arrive and when blocks become "proven"

**Reference:** Kev's `events.rs` (+16 lines), `types.rs` (+31 lines)

---

#### Task 17: HTTP API endpoints for proof status
**Files to modify:**
- `beacon_node/http_api/src/lib.rs`
- `beacon_node/http_api/src/publish_blocks.rs`

**Details:**
- Add endpoint to query proof status of a block
- Add endpoint to manually submit execution proofs (for testing)
- Metrics: proof count per block, proof latency, proven chain head vs optimistic head

---

### Phase 7: Testing and Integration (2-3 sessions)

#### Task 18: Unit tests for proof verification and DA checker
**Files to modify:**
- `beacon_node/beacon_chain/tests/` (new test files)

**Details:**
- Test proof validation logic
- Test DA checker with execution proof requirements
- Test `make_available` threshold behavior
- Test proof deduplication by subnet_id

---

#### Task 19: Kurtosis testnet with stateless nodes
**Files to modify:**
- `scripts/local_testnet/` or kurtosis configs

**Details:**
- Configure a local testnet with mixed nodes:
  - 2 regular nodes (with EL)
  - 1 proof generator node
  - 1 stateless validator node
- Verify stateless node can follow chain via proofs
- Test with ePBS flow (builder reveals + proof publication)

---

#### Task 20: Real zkEVM prover integration (future)
**Details:**
- Replace stub proof generation with actual zkEVM prover
- Candidates: SP1 (Succinct), RISC Zero, Jolt
- SP1 is currently most mature for Ethereum block proving (~minutes per block on GPU)
- This task is intentionally last — all infrastructure should work with stubs first

## Current State of zkEVM Proving Systems

| System | Organization | Approach | Ethereum Block Proving | Maturity |
|--------|-------------|----------|----------------------|----------|
| **SP1** | Succinct | RISC-V zkVM | Active development, demonstrated mainnet block proving | Production-ready for some use cases |
| **RISC Zero** | RISC Zero | RISC-V zkVM | Supports Ethereum block proving | Production-ready |
| **Jolt** | a16z | RISC-V zkVM (Lasso lookup) | Earlier stage | Research/development |
| **Zeth** | RISC Zero | reth inside RISC Zero zkVM | Proven mainnet blocks | Demonstrated |
| **SP1-reth** | Succinct | reth inside SP1 | Proven mainnet blocks | Demonstrated |

**Current proving times** (as of early 2025):
- GPU-accelerated: ~30s to ~5min per mainnet block (depends on gas usage)
- Target for "real-time proving": <12s per block (one slot)
- ePBS advantage: builder can start proving before the slot, gaining minutes of headroom

## Open Questions and Decisions

### Architecture
1. **Should proofs be bundled with `ExecutionPayloadEnvelope`?**
   - Pro: Atomic delivery, builder proves commitment
   - Con: Increases envelope size, delays reveal, not in current spec
   - Current spec (EIP-8025): Proofs are separate, on gossip subnets

2. **Should the bid signal proof availability?**
   - Adding a flag to `ExecutionPayloadBid` ("I will provide a ZK proof") could let proposers prefer proving builders
   - Requires spec change

3. **How many proof types to support initially?**
   - Start with 1 (stubbed), expand to SP1 as first real prover
   - Subnet allocation strategy: sequential vs random

4. **Proof chain integration with fork choice?**
   - Current design: proof status is metadata only, doesn't affect fork choice
   - Future: could add soft preference for proven blocks (requires careful analysis)

### ePBS-Specific
5. **When does the builder publish proofs relative to the envelope?**
   - Before envelope: proof arrives first, envelope triggers block availability
   - With envelope: bundled delivery
   - After envelope: proof arrives on subnets, block transitions from optimistic to proven

6. **Can the PTC (Payload Timeliness Committee) consider proof status?**
   - Currently PTC only checks timeliness
   - Future: PTC could require proof for "full" attestation

### Engineering
7. **Proof storage and persistence?**
   - Kev's PR: proofs follow block lifecycle, LRU eviction
   - Do we need to persist proofs in DB for RPC serving?

8. **How to handle proof generation failure?**
   - Builder can't produce proof in time → block remains optimistic
   - Invalid proof → reject and penalize? (no slashing mechanism yet)

9. **Backward compatibility with non-ePBS Lighthouse?**
   - vibehouse is a fork, so less concern
   - But EIP-8025 consensus-specs PR is designed to be fork-agnostic

10. **GPU requirements for proof generation?**
    - Stub first, real proving requires CUDA/Metal
    - Should be opt-in only (`--generate-execution-proofs`)

## References

- **Kev's Lighthouse PR**: https://github.com/sigp/lighthouse/pull/7755
  - Pre-ePBS prototype, 1824 additions across 40 files
  - Key files: execution_proof.rs, execution_proof_verification.rs, execution_proof_generation.rs, DA checker changes
- **EIP-8025 Consensus Specs**: https://github.com/ethereum/consensus-specs/pull/4591
  - Merged spec for optional execution proofs
- **HackMD Notes**: https://hackmd.io/@kevaundray/BJeZCo5Tgx
  - Comprehensive architecture document covering CL/EL primer, stateless vs executionless, dual-view design
- **vibehouse Issue #28**: ZK proofs alongside ePBS envelopes
- **EIP-7732 (ePBS)**: Enshrined Proposer-Builder Separation
- **vibehouse ePBS types**:
  - `consensus/types/src/execution_payload_envelope.rs` — Builder's payload reveal
  - `consensus/types/src/execution_payload_bid.rs` — Builder's bid commitment
  - `beacon_node/beacon_chain/src/gloas_verification.rs` — ePBS verification logic
- **Ethereum Roadmap - The Verge**: Stateless validation via Verkle trees and ZK proofs
- **SP1 (Succinct)**: https://github.com/succinctlabs/sp1
- **RISC Zero**: https://github.com/risc0/risc0
- **Jolt**: https://github.com/a16z/jolt
- **Zeth (reth in RISC Zero)**: https://github.com/risc0/zeth
