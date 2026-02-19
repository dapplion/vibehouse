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

### 2026-02-19 — Task 14: stateless validation bypass (Phase 5 continued)
- **Added stateless bypass to `process_payload_envelope()`** — when `self.config.stateless_validation` is true, the entire EL `notify_new_payload` call is skipped. The block stays optimistic until sufficient execution proofs arrive via gossip. The envelope state transition, proof generation trigger, state caching, and disk persistence all still execute normally.
- **Added stateless bypass to `process_self_build_envelope()`** — same pattern. Although stateless nodes typically don't self-build (no EL), the guard is present for completeness and mixed-mode configurations.
- **Added stateless bypass to `PayloadNotifier::new()`** — for pre-Gloas execution-enabled blocks, returns `PayloadVerificationStatus::Optimistic` when `stateless_validation` is true, avoiding the `NoExecutionConnection` error that would fire when no EL is configured.
- **Added stateless bypass to `update_execution_engine_forkchoice()`** — returns `Ok(())` early, skipping `forkchoiceUpdated` calls to the EL since there is no EL to call.
- **Added stateless bypass to `prepare_beacon_proposer()`** — returns `Ok(None)` early, skipping proposer preparation since stateless nodes don't produce blocks.
- **Design decisions**: The bypasses are minimal `if self.config.stateless_validation` guards that return early or skip the EL block, preserving all other logic (state transitions, proof generation, fork choice updates, etc). No new error variants or types needed. The `canonical_head.rs` callers of both `update_execution_engine_forkchoice` and `prepare_beacon_proposer` handle the early-return `Ok` values correctly (they log `crit!` on errors but our returns are success values).
- **Files changed**: 2 modified
  - `beacon_node/beacon_chain/src/beacon_chain.rs`: 4 stateless guards (~+20 lines)
  - `beacon_node/beacon_chain/src/execution_payload.rs`: 1 stateless guard (~+5 lines)
- 309/309 beacon_chain tests pass (Gloas fork), clippy clean, cargo fmt clean, full lighthouse binary builds.

### 2026-02-19 — Task 13: proof broadcaster service (Phase 5 continued)
- **Created `proof_broadcaster.rs` in the client crate** — background async service that consumes proofs from the `ProofReceiver` channel and publishes each as a `PubsubMessage::ExecutionProof` via `NetworkMessage::Publish`. Simple loop: `recv()` → wrap in `PubsubMessage::ExecutionProof(Box::new((subnet_id, proof)))` → send to network. Stops gracefully when either the generator channel closes or the network sender is dropped.
- **Added `proof_receiver` field to `BeaconChain`** — `Mutex<Option<ProofReceiver>>`, stored alongside the generator. The `Mutex<Option<_>>` pattern allows the client builder to `.take()` the receiver once at startup for the broadcaster service, while the `BeaconChain` remains in an `Arc`.
- **Updated `builder.rs` (beacon_chain)** — now stores the `proof_rx` in the `BeaconChain` struct instead of discarding it with `_proof_rx`. Removed the `TODO(vibehouse#28)` comment.
- **Wired broadcaster spawn in `client/src/builder.rs`** — in the `build()` method, after all other services are started, checks if `proof_receiver` is available (via `lock().take()`). If so, and if `network_senders` is available, spawns `run_proof_broadcaster` as a named task (`"proof_broadcaster"`).
- **Design decisions**: The broadcaster lives in the `client` crate (not `beacon_chain`) because it needs `NetworkMessage` and `PubsubMessage` types from the `network` and `lighthouse_network` crates. This follows the same pattern as `SlasherService` which also lives in a separate crate and receives a network sender handle. The broadcaster is intentionally simple — no retry logic, no batching — because the unbounded channel provides backpressure-free delivery and gossipsub handles retransmission.
- **Files changed**: 5 (1 new, 4 modified)
  - `beacon_node/client/src/proof_broadcaster.rs` (new, ~44 lines)
  - `beacon_node/client/src/lib.rs` (module declaration)
  - `beacon_node/client/src/builder.rs` (spawn broadcaster, ~+11 lines)
  - `beacon_node/beacon_chain/src/beacon_chain.rs` (proof_receiver field, ~+3 lines)
  - `beacon_node/beacon_chain/src/builder.rs` (store proof_rx instead of dropping, ~+4/-7 lines)
- 309/309 beacon_chain tests pass (Gloas fork), clippy clean, cargo fmt clean, full release binary builds.

### 2026-02-19 — Task 12: proof generation trigger (Phase 5 started)
- **Created `execution_proof_generation.rs`** — new module with `ExecutionProofGenerator` struct. Uses an unbounded `mpsc` channel to emit generated proofs for downstream consumption (Task 13 broadcaster). `generate_proof(block_root, block_hash)` creates one stub proof per subnet (currently subnet 0 only, since `MAX_EXECUTION_PROOF_SUBNETS = 1`). Stub proof data is `b"vibehouse-stub-proof-v1"` — real ZK prover integration is Task 20. Two unit tests: `generates_stub_proof_for_each_subnet` and `proof_receiver_dropped_does_not_panic`.
- **Added `execution_proof_generator` field to `BeaconChain`** — `Option<ExecutionProofGenerator>`, only `Some` when `--generate-execution-proofs` is enabled. Initialized in `builder.rs` with the channel receiver stored as `_proof_rx` (Task 13 will wire it into a broadcaster service).
- **Wired proof generation into both ePBS payload processing paths**:
  1. `process_payload_envelope()` (gossip path) — triggered after `execution_layer.notify_new_payload()` succeeds, before the envelope state transition. Uses `signed_envelope.message.payload.block_hash` for the proof.
  2. `process_self_build_envelope()` (self-build path) — triggered after `notify_new_payload()` succeeds, before state transition. Uses `payload_block_hash` extracted earlier in the method.
- **Design decisions**: Proof generation is synchronous (stub is instant; real ZK will be async via `spawn_blocking` in Task 20). Trigger point is after EL validation succeeds but before state transition — this ensures we only generate proofs for payloads the EL considers valid. Both gossip and self-build paths trigger proofs so builders using the self-build flow also generate proofs.
- **Files changed**: 4 (1 new, 3 modified)
  - `beacon_node/beacon_chain/src/execution_proof_generation.rs` (new, ~123 lines)
  - `beacon_node/beacon_chain/src/lib.rs` (module declaration)
  - `beacon_node/beacon_chain/src/beacon_chain.rs` (field + two trigger sites, ~+18 lines)
  - `beacon_node/beacon_chain/src/builder.rs` (initialization, ~+10 lines)
- 309/309 beacon_chain tests pass (Gloas fork), 2/2 new unit tests pass, clippy clean, cargo fmt clean.

### 2026-02-19 — Tasks 10-11: gossip processing and router wiring (Phase 4 complete)
- **Added `GossipExecutionProof` work type to beacon processor** — new `Work::GossipExecutionProof(AsyncFn)` variant with `WorkType::GossipExecutionProof`, dedicated FIFO queue (capacity 4096), queue dispatch, priority ordering (after gossip_proposer_preferences, before RPC methods), and async task spawning (same group as GossipExecutionPayload/GossipPayloadAttestation).
- **Added `process_gossip_execution_proof()` to `gossip_methods.rs`** — public async method that receives an `Arc<ExecutionProof>` + subnet_id from the router. Calls `chain.verify_execution_proof_for_gossip()` for validation, then on success calls `process_gossip_verified_execution_proof()` for DA checker import. Error handling follows the data column pattern: UnknownBlockRoot → Ignore (no penalty), PriorToFinalization → Ignore + HighTolerance penalty, all structural/crypto errors → Reject + LowTolerance penalty, BeaconChainError → crit! (no penalty).
- **Added `process_gossip_verified_execution_proof()`** — async helper that looks up the block slot from fork choice (ExecutionProof has no slot field), then calls `chain.check_gossip_execution_proof_availability_and_import()`. Handles Imported (recompute head + notify sync), MissingComponents (trace log), DuplicateFullyImported (debug log), and errors (MidTolerance penalty).
- **Added `send_gossip_execution_proof()` to `network_beacon_processor/mod.rs`** — creates a `Work::GossipExecutionProof` event with `drop_during_sync: false` (proofs are data-integrity critical, not dropped during sync).
- **Wired router dispatch** — replaced the `PubsubMessage::ExecutionProof` debug placeholder in `router.rs` with full dispatch to `send_gossip_execution_proof`, destructuring the `(subnet_id, proof)` tuple from the boxed message.
- **Phase 4 (Network Processing) is now complete**: Tasks 10-11 done. The full gossip pipeline from network → router → beacon processor → gossip verification → DA checker → block import is wired.
- **Files changed**: 4 modified
  - `beacon_node/beacon_processor/src/lib.rs`: Work/WorkType variants, queue, priority, dispatch (~+16 lines)
  - `beacon_node/network/src/network_beacon_processor/gossip_methods.rs`: two new methods + imports (~+178 lines)
  - `beacon_node/network/src/network_beacon_processor/mod.rs`: send method (~+28 lines)
  - `beacon_node/network/src/router.rs`: dispatch wiring (~+14 lines, -4 lines placeholder)
- 96/96 network tests pass (Gloas fork), 307/307 beacon_chain tests pass (Gloas fork), clippy clean, cargo fmt clean, full release binary builds.

### 2026-02-19 — Task 9: beacon chain proof intake methods (Phase 3 complete)
- **Added `check_gossip_execution_proof_availability_and_import()` to `BeaconChain`** — public async method following the `check_gossip_data_columns_availability_and_import()` pattern. Accepts `VerifiedExecutionProof<T>`, extracts subnet_id and inner proof, delegates to `data_availability_checker.put_gossip_verified_execution_proofs()`, then calls `process_availability()` to handle the result (import if fully available, return MissingComponents otherwise).
- **Wired `stateless_min_proofs_required` from `ChainConfig` through builder into DA checker** — added `min_execution_proofs_required: usize` field to `DataAvailabilityCheckerInner`, passed through `DataAvailabilityChecker::new()` from builder. In builder, value is `chain_config.stateless_min_proofs_required` when `stateless_validation` is enabled, 0 otherwise. This means non-stateless nodes don't gate on proofs (they verify via EL directly).
- **Updated `min_execution_proofs_for_epoch()`** to use the configurable field instead of hardcoded `MAX_EXECUTION_PROOF_SUBNETS`. This fixes a latent issue: previously all Gloas blocks required proofs even for nodes with EL access.
- **No new error variants needed** — existing `AvailabilityCheckError` (already in `BeaconChainError`) and `GossipExecutionProofError` cover all cases.
- **Phase 3 (Proof Verification and Storage) is now complete**: Tasks 7-9 done.
- **Files changed**: 4 modified
  - `beacon_node/beacon_chain/src/beacon_chain.rs`: new method + import (~+20 lines)
  - `beacon_node/beacon_chain/src/builder.rs`: compute and pass min_execution_proofs_required (~+5 lines)
  - `beacon_node/beacon_chain/src/data_availability_checker.rs`: new constructor param (~+2 lines)
  - `beacon_node/beacon_chain/src/data_availability_checker/overflow_lru_cache.rs`: new field, constructor param, configurable threshold (~+6 lines)
- 307/307 beacon_chain tests pass (Gloas fork), clippy clean, cargo fmt clean, full binary builds.

### 2026-02-19 — Task 8: integrate proofs into DataAvailabilityChecker
- **Added `verified_execution_proofs` field to `PendingComponents`** — `HashMap<ExecutionProofSubnetId, Arc<ExecutionProof>>`, initialized empty. Keyed by subnet_id so duplicates are silently deduplicated.
- **Added `merge_execution_proofs()` method** to `PendingComponents` — inserts proofs, skipping duplicates via `entry().or_insert()`.
- **Added execution proof gate in `make_available()`** — after the blob/column data gate, checks that `verified_execution_proofs.len() >= min_execution_proofs_required`. For pre-Gloas blocks this threshold is 0 (no proofs needed). For Gloas+ blocks it equals `MAX_EXECUTION_PROOF_SUBNETS` (currently 1).
- **Added `min_execution_proofs_for_epoch()` helper** to `DataAvailabilityCheckerInner` — returns `MAX_EXECUTION_PROOF_SUBNETS` for Gloas+ epochs, 0 otherwise. Uses `spec.fork_name_at_epoch()` for clean fork detection.
- **Added `put_execution_proofs()` inner method** on `DataAvailabilityCheckerInner` — follows the `put_kzg_verified_data_columns()` pattern: peek epoch from cache, update_or_insert pending components, check availability.
- **Added `put_gossip_verified_execution_proofs()` outer method** on `DataAvailabilityChecker` — thin wrapper delegating to inner method.
- **Design decision**: Proofs are "gate" data only — required for block availability but NOT added to `AvailableBlock::deconstruct()`. This avoids breaking ~20 callers. The proof data is consumed from the cache when needed (e.g., for RPC serving in a future task).
- **Files changed**: 2
  - `beacon_node/beacon_chain/src/data_availability_checker/overflow_lru_cache.rs` (~+70 lines)
  - `beacon_node/beacon_chain/src/data_availability_checker.rs` (~+15 lines)
- Clippy clean, cargo fmt clean.

### 2026-02-19 — Task 7: proof verification module (Phase 3 started)
- **Created `execution_proof_verification.rs`** — new gossip verification module following the `gloas_verification.rs` pattern (simpler than blob/column since no proposer equivocation or KZG concerns).
- **`GossipExecutionProofError`** enum with 8 variants:
  - Reject: `InvalidSubnetId`, `InvalidVersion`, `ProofDataEmpty`, `ProofDataTooLarge`, `BlockHashMismatch`, `InvalidProof`
  - Ignore: `UnknownBlockRoot`, `PriorToFinalization`
  - Internal: `BeaconChainError`
- **`VerifiedExecutionProof<T>`** wrapper type with `proof()`, `block_root()`, `subnet_id()`, `into_inner()` accessors.
- **`verify_execution_proof_for_gossip()`** — 7-step validation:
  1. Subnet ID bounds check
  2. Version supported check
  3. Structural validity (non-empty, size limits)
  4. Block root known in fork choice (read lock)
  5. Block not prior to finalization
  6. Block hash matches bid block hash (Gloas ePBS cross-check)
  7. Cryptographic verification (stubbed — returns Ok always, real ZK verification in later task)
- **`BeaconChain::verify_execution_proof_for_gossip()`** thin wrapper method for the network layer to call.
- **Files changed**: 2 (1 new, 1 modified)
  - `beacon_node/beacon_chain/src/execution_proof_verification.rs` (new, ~210 lines)
  - `beacon_node/beacon_chain/src/lib.rs` (module declaration)
- 307/307 beacon_chain tests pass (Gloas fork), 1/1 new unit test passes.

### 2026-02-19 — Task 6: gossip subscription and whitelist (Phase 2 complete)
- **Added `ExecutionProof` subnets to gossipsub whitelist filter** in `create_whitelist_filter()` (`service/utils.rs`). Without this, gossipsub's `WhitelistSubscriptionFilter` silently rejects incoming `execution_proof_N` messages even when subscribed. Loops over `0..MAX_EXECUTION_PROOF_SUBNETS` adding each proof topic hash.
- **Also added missing `ProposerPreferences`** to the whitelist — this ePBS topic was omitted when the whitelist was first set up.
- **No additional peer management changes needed**: Subscription lifecycle is already fully wired from Task 5 (`core_topics_to_subscribe` → `subscribe_new_fork_topics`). Gossip cache policy (`None`) was set in Task 4. Peer tracking works through standard gossipsub subscription tracking.
- **Design note**: `good_peers_on_subnet(Subnet::ExecutionProof(_))` always returns 0 (since `on_subnet_metadata` returns `false` — no metadata/ENR field). This means discovery is always triggered, but since `subnet_predicate` accepts any fork-matching peer, this is harmless and self-correcting. A dedicated `maintain_execution_proof_peers` heartbeat was not added — proof subnets are permanent (not duty-scheduled like sync committees), so the mesh naturally populates through gossipsub.
- **Phase 2 (Network Layer) is now complete**: Tasks 4-6 done. Execution proof messages can be encoded/decoded (Task 4), peers can be discovered on proof subnets (Task 5), and the gossipsub whitelist accepts proof topics (Task 6).
- **Files changed**: 1 modified
  - `beacon_node/lighthouse_network/src/service/utils.rs`: whitelist filter additions
- 92/92 lighthouse_network tests pass, 96/96 network tests pass.

### 2026-02-19 — Task 5: execution proof subnet discovery
- **Added `Subnet::ExecutionProof(ExecutionProofSubnetId)` variant** to the `Subnet` enum, enabling execution proof subnets to participate in the discovery and peer management systems.
- **Subnet predicate**: Execution proof subnets accept all peers (no ENR filtering). Any peer at the correct fork can serve proofs.
- **ENR**: No new ENR field added — execution proof subnet membership is implicit (like data column subnets being computed, proof subnets are opt-in via config). `update_enr_bitfield` returns `Ok(())` for proof subnets.
- **Discovery metrics**: Added `"execution_proof"` label for subnet query metrics.
- **Peer management**: `on_subnet_metadata` returns `false` for proof subnets (no metadata tracking). Long-lived subnet info ignores proof subnets.
- **TopicConfig**: Added `subscribe_execution_proof_subnets: bool` field. When `true`, `core_topics_to_subscribe` adds `ExecutionProof` topics for all subnets (0..MAX_EXECUTION_PROOF_SUBNETS) at Gloas fork.
- **Network config**: Added `subscribe_execution_proof_subnets: bool` (default `false`). Wired from CLI: set to `true` when `--stateless-validation` or `--generate-execution-proofs` is enabled.
- **GossipTopic↔Subnet conversions**: `subnet_id()` returns `Subnet::ExecutionProof` for proof topics. `From<Subnet> for GossipKind` maps to `GossipKind::ExecutionProof`.
- **Files changed**: 8 modified
  - `beacon_node/lighthouse_network/src/types/subnet.rs`: ExecutionProof variant
  - `beacon_node/lighthouse_network/src/types/topics.rs`: TopicConfig field, core_topics, all_topics, subnet_id, From impl
  - `beacon_node/lighthouse_network/src/types/globals.rs`: as_topic_config wiring
  - `beacon_node/lighthouse_network/src/discovery/subnet_predicate.rs`: ExecutionProof match arm
  - `beacon_node/lighthouse_network/src/discovery/mod.rs`: update_enr_bitfield, metrics
  - `beacon_node/lighthouse_network/src/peer_manager/peerdb/peer_info.rs`: on_subnet_metadata
  - `beacon_node/lighthouse_network/src/peer_manager/mod.rs`: long-lived subnet info
  - `beacon_node/lighthouse_network/src/config.rs`: subscribe_execution_proof_subnets field + default
  - `beacon_node/src/config.rs`: CLI wiring
- 92/92 lighthouse_network tests pass, 96/96 network tests pass, 311/311 types tests pass.

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
