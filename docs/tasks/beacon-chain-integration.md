# Beacon Chain Integration (Phase 5)

## Objective
Wire up gloas ePBS types through the beacon chain crate — block import pipeline, fork choice store, two-phase block handling, payload timeliness committee.

## Status: IN PROGRESS

### Done
- ✅ Block verification pipeline for gloas (bid KZG check, parent root validation, availability handling)
- ✅ Payload envelope gossip verification + fork choice (signature sets, error types, gossip handler)
- ✅ Self-build block production (replaced `GloasNotImplemented` with real implementation)
- ✅ Envelope processing state transition (`process_execution_payload_envelope` — 273 lines, spec-compliant)
- ✅ Envelope processing wired into beacon chain import pipeline
- ✅ EL integration: `newPayload` call for gloas payloads via envelope processing
- ✅ Envelope creation and broadcast after block production (self-build flow)
- ✅ Chain head recompute after payload reveal and PTC attestations
- ✅ Payload attestation pool for block inclusion (gossip → pool → block production)
- ✅ Gossip topic names fixed to match spec (`execution_payload_bid`, `payload_attestation_message`)
- ✅ Code cleanup: removed unused imports and dead `GloasNotImplemented` error variant
- ✅ Gloas preset values: populated GloasPreset struct, fixed minimal spec (PTC_SIZE=2, MAX_BUILDERS_PER_WITHDRAWALS_SWEEP=16)

### Remaining
- [ ] Handle the two-phase block: external builder path (proposer commits to external bid, builder reveals)
- [ ] `ProposerPreferences` gossip topic (not needed for devnet-0 self-build)
- [x] Implement payload timeliness committee logic (PTC attestation pool + block inclusion)
- [x] Update `CachedHead.head_hash` for ePBS (EL execution_status after envelope)

## Key files
- `beacon_node/beacon_chain/src/beacon_chain.rs` — block production, fork choice bridge
- `consensus/state_processing/src/envelope_processing.rs` — envelope state transition
- `beacon_node/beacon_chain/src/block_verification.rs` — block import pipeline
- `beacon_node/beacon_chain/src/execution_payload.rs` — EL integration
- `beacon_node/beacon_chain/src/gloas_verification.rs` — gossip verification

## Progress log

### 2026-02-17 — Populate gloas preset values and fix minimal spec
- **Bug**: `GloasPreset` struct was empty — preset YAML files had no values, and the minimal chain spec inherited mainnet values (PTC_SIZE=512, MAX_BUILDERS_PER_WITHDRAWALS_SWEEP=16384) which are wrong for devnet-0
- **Fix 1**: Added 5 fields to `GloasPreset` struct matching consensus-specs preset: `ptc_size`, `max_payload_attestations`, `builder_registry_limit`, `builder_pending_withdrawals_limit`, `max_builders_per_withdrawals_sweep`
- **Fix 2**: `ChainSpec::minimal()` now overrides `ptc_size: 2` and `max_builders_per_withdrawals_sweep: 16` (vs mainnet 512/16384)
- **Fix 3**: Updated all three preset YAML files (mainnet, minimal, gnosis) with actual values from consensus-specs
- Without this fix, devnet-0 (minimal preset) would use wrong PTC quorum threshold (256 instead of 1) in fork choice
- All preset consistency tests pass (mainnet, minimal, gnosis)
- Pre-existing EF test results unchanged (86/87 pass, data_column_sidecar SSZ failure pre-existing)

### 2026-02-17 — Fix gossip topic names + code cleanup
- **Bug**: Gossip topic names didn't match consensus spec v1.7.0-alpha.2
  - `execution_bid` → `execution_payload_bid` (spec name)
  - `payload_attestation` → `payload_attestation_message` (spec name)
  - `execution_payload` was already correct
- This would have prevented cross-client interop in devnet-0
- Also removed unused imports (`AvailableBlockData`, `FixedBytesExtended`, `MessageAcceptance`) and dead `GloasNotImplemented` error variant
- Pre-existing EF test results unchanged (86/87 pass, data_column_sidecar SSZ failure pre-existing)
- Commits: `590a9a238`, `1ee5572e0`

### 2026-02-17 — Payload attestation pool for block inclusion
- **Problem**: Verified PTC attestations from gossip were applied to fork choice but discarded — blocks were produced with empty `payload_attestations` list
- **Fix**: Added `payload_attestation_pool` (HashMap<Slot, Vec<PayloadAttestation>>) on BeaconChain
- Gossip handler now inserts verified attestations into the pool after verification
- Block production retrieves attestations for `slot - 1` from the pool, limited to `max_payload_attestations` (4 mainnet, 2 minimal)
- Pool auto-prunes entries older than 2 epochs to bound memory
- Pre-existing EF test results unchanged (86/87 pass, data_column_sidecar SSZ failure pre-existing)
- Commit: `028ce5264`

### 2026-02-17 — Set execution block hash in fork choice for gloas ePBS blocks
- **Problem**: Gloas blocks contain bids (not payloads), so `execution_status` was `Irrelevant` and `head_hash` was `None` when `forkchoice_updated` was sent to the EL
- **Fix 1**: `on_block()` now sets `execution_status = Optimistic(bid.block_hash)` for gloas blocks, so self-build blocks have `head_hash` immediately when elected as head
- **Fix 2**: `on_execution_payload()` now accepts `payload_block_hash` parameter and updates `execution_status` when the payload envelope is revealed
- **Fix 3**: `on_payload_attestation()` sets `execution_status` from `bid_block_hash` when PTC quorum marks payload as revealed (fallback if envelope path hasn't set it)
- All three paths ensure `head_hash` is available for `forkchoice_updated` calls to the EL
- Pre-existing EF test results unchanged (87/88 pass, data_column_sidecar SSZ failure pre-existing)
- Commit: `2707325ab`

### 2026-02-17 — Chain head recompute after payload reveal and PTC attestations
- Added `recompute_head_at_current_slot()` call after successful envelope fork choice import in `process_gossip_execution_payload`
- Added `recompute_head_at_current_slot()` call after successful PTC attestation fork choice import in `process_gossip_payload_attestation`
- Changed `GossipPayloadAttestation` work type from `BlockingFn` to `AsyncFn` to support the async head recompute call
- Without this, a block whose payload was just revealed (via envelope or PTC quorum) would not become the chain head until the next periodic recompute tick
- Pre-existing EF test results unchanged (87/88 pass, data_column_sidecar SSZ failure pre-existing)
- Commit: `b5f9af3a3`

### 2026-02-17 — Self-build envelope creation and broadcast
- During block production (`produce_block_on_state`), extracts `ExecutionPayloadGloas` and `ExecutionRequests` from `BlockProposalContents` before it's consumed
- `build_self_build_envelope()` computes the post-envelope state root by running `process_execution_payload_envelope` on a cloned post-block state with a placeholder state_root, then extracts the actual root from the `InvalidStateRoot` error
- Envelope cached in `BeaconChain::pending_self_build_envelope` (new field)
- After block import succeeds in `publish_block`, the pending envelope is broadcast via `PubsubMessage::ExecutionPayload`
- Added `execution_payload_envelope` field to `BeaconBlockResponse` for future use
- Signature is empty (infinity point) for `BUILDER_INDEX_SELF_BUILD`
- Pre-existing EF test results unchanged (86/87 pass, data_column_sidecar SSZ failure pre-existing)
- Commit: `8cccdb5a8`

### 2026-02-17 — newPayload EL call wired into envelope processing
- `process_payload_envelope` now async: sends `engine_newPayloadV4` to EL before state transition
- Constructs `NewPayloadRequestGloas` from envelope payload + bid's `blob_kzg_commitments` + block's `parent_root`
- Handles all EL responses: Valid, Syncing/Accepted (optimistic), Invalid, InvalidBlockHash
- `GossipExecutionPayload` work type changed from `BlockingFn` to `AsyncFn` to support async EL call
- Gossip handler `process_gossip_execution_payload` made async
- Pre-existing EF test results unchanged (87/88 pass, data_column_sidecar SSZ failure pre-existing)
- Commit: `36b19e070`

### 2026-02-17 — Envelope processing wired into import pipeline
- Recovered orphaned `envelope_processing.rs` from commit `1844ddfb0` (was lost when `daa27499e` was committed as sibling instead of child)
- Added `BeaconChain::process_payload_envelope()`: retrieves head state, applies `process_execution_payload_envelope` state transition
- Updated gossip handler to call `process_payload_envelope` after fork choice update
- Added `EnvelopeProcessingError` variant to `BeaconChainError`
- 77/77 EF tests passing
- Commit: `4daffbe2e`

### 2026-02-16 — Block production + envelope processing
- Implemented self-build block production: creates `ExecutionPayloadBid` from EL response, uses `BUILDER_INDEX_SELF_BUILD` (u64::MAX) with value=0, empty signature
- Implemented `process_execution_payload_envelope`: full spec-compliant validation (beacon block root match, slot match, builder index, prev_randao, withdrawals, gas limit, block hash, parent hash, timestamp), processes execution requests, builder payment, sets availability flag
- Commits: `fd9c93264`, `1844ddfb0`

### 2026-02-15 — Gossip verification + fork choice
- Added `execution_payload_envelope_signature_set()` using `DOMAIN_BEACON_BUILDER`
- `PayloadEnvelopeError` with 9 variants and peer scoring
- `verify_payload_envelope_for_gossip()` — 6 validation checks
- `on_execution_payload()` in fork choice — marks `payload_revealed = true`
- Wired up `send_gossip_execution_payload()` handler
- Commit: `35b99ae6a`

### 2026-02-15 — Block verification pipeline
- Gossip verification checks `signed_execution_payload_bid.blob_kzg_commitments` for gloas
- Gloas blocks marked as `MaybeAvailableBlock::Available` with `NoData`
- `PayloadNotifier` returns `Irrelevant` for gloas
- DA cache skip for gloas blocks
- Commit: `18fc434ac`
