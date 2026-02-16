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

### Remaining
- [ ] EL integration: `newPayload` call for gloas payloads
- [ ] Envelope creation and broadcast after block production (self-build flow)
- [ ] Update chain head tracking for ePBS
- [ ] Handle the two-phase block: proposer commits, builder reveals
- [ ] Implement payload timeliness committee logic

## Key files
- `beacon_node/beacon_chain/src/beacon_chain.rs` — block production, fork choice bridge
- `consensus/state_processing/src/envelope_processing.rs` — envelope state transition
- `beacon_node/beacon_chain/src/block_verification.rs` — block import pipeline
- `beacon_node/beacon_chain/src/execution_payload.rs` — EL integration
- `beacon_node/beacon_chain/src/gloas_verification.rs` — gossip verification

## Progress log

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
