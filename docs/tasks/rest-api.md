# REST API (Phase 7)

## Objective
Add ePBS-specific REST API endpoints for block submission, bid submission, payload attestation, and SSE events.

## Status: DONE

### Done
- [x] SSZ response support for 6 endpoints (#8892)
- [x] SSE events for ePBS: `execution_bid`, `execution_payload`, `payload_attestation`
- [x] `GET /eth/v1/beacon/states/{state_id}/proposer_lookahead` — Fulu/Gloas only, returns `proposer_lookahead` vector
- [x] `POST /eth/v1/builder/bids` — accepts `SignedExecutionPayloadBid`, verifies, imports to fork choice, gossips
- [x] `POST /eth/v1/beacon/execution_payload_envelope` — accepts `SignedExecutionPayloadEnvelope`, verifies (gossip checks), gossips on `execution_payload`, runs state transition (newPayload + process_execution_payload)
- [x] Envelope DB persistence: `DBColumn::BeaconEnvelope` + `StoreOp::PutPayloadEnvelope` / `DeletePayloadEnvelope` + pruning on finalization
- [x] `GET /eth/v1/beacon/execution_payload_envelope/{block_id}` — returns stored `SignedExecutionPayloadEnvelope` for any block
- [x] Blinded blocks endpoint: verified working for Gloas — `try_into_full_block(None)` is correct because Gloas blocks have no execution payload in the body (the Blinded/Full distinction is purely a phantom type marker)
- [x] `GET /eth/v1/beacon/pool/payload_attestations?slot` — returns all payload attestations from pool, optional slot filter

## Progress log

### 2026-02-17 — SSE events for ePBS gossip messages
- **Problem**: No SSE events were emitted for ePBS-specific gossip messages (execution bids, payload envelopes, payload attestations). Devnet monitoring tools like Dora subscribe to SSE events to display chain activity — without these events, ePBS activity would be invisible in the dashboard.
- **New SSE event types**:
  - `execution_bid`: Emitted when a verified execution bid is imported to fork choice. Fields: slot, block (parent_block_root), builder_index, block_hash, value.
  - `execution_payload`: Emitted when a payload envelope is successfully processed (EL notified via newPayload + state transition complete). Fields: slot, beacon_block_root, builder_index, block_hash.
  - `payload_attestation`: Emitted when a verified PTC attestation is imported to fork choice. Fields: slot, beacon_block_root, payload_present, blob_data_available.
- **Files changed**: 4 files
  - `common/eth2/src/types.rs`: Added `SseExecutionBid`, `SseExecutionPayload`, `SsePayloadAttestation` structs + `EventKind` variants + `EventTopic` variants + parsing
  - `beacon_node/beacon_chain/src/events.rs`: Added 3 broadcast channels with subscribe/has_subscribers methods
  - `beacon_node/network/src/network_beacon_processor/gossip_methods.rs`: Emit events in ePBS gossip handlers after successful verification/import
  - `beacon_node/http_api/src/lib.rs`: Wired up subscriptions in `/eth/v1/events` endpoint
- 136/136 EF tests pass, check_all_files_accessed passes, clippy clean

### 2026-02-18 — proposer lookahead endpoint
- **Endpoint**: `GET /eth/v1/beacon/states/{state_id}/proposer_lookahead`
- **What it does**: Returns the raw `proposer_lookahead` vector from the Fulu/Gloas beacon state — a list of validator indices (one per slot) covering the current and next epoch's proposer schedule. Pre-Fulu states return 400.
- **Why**: Upstream PR sigp/lighthouse#8815 (per beacon-APIs#565). External tooling (MEV relays, block explorers) can use this to know proposer assignments without re-computing the shuffling.
- **Response**: `{ "execution_optimistic": bool, "finalized": bool, "data": [u64, ...] }` (no version header since data is unversioned)
- **Files changed**: 2 files
  - `beacon_node/http_api/src/lib.rs`: Added route following `pending_consolidations` pattern; `ResponseIncludesVersion::No` since data is raw u64 vector
  - `common/eth2/src/lib.rs`: Added `get_beacon_states_proposer_lookahead` client method returning `UnversionedResponse<Vec<u64>, ExecutionOptimisticFinalizedMetadata>`
- 181/181 http_api tests pass

### 2026-02-18 — execution bid submission endpoint
- **Endpoint**: `POST /eth/v1/builder/bids`
- **What it does**: External builders (or any node) can submit a `SignedExecutionPayloadBid` via HTTP. The BN verifies it (same checks as gossip: slot, payment, builder active, signature, no equivocation), imports to fork choice, and gossips on the `execution_bid` P2P topic.
- **Error handling**: Duplicate bids → 200 (idempotent); equivocation, invalid sig, unknown builder, etc. → 400.
- **Fork guard**: Returns 400 "Gloas is not scheduled" if called pre-Gloas.
- **Files changed**: 2 files
  - `beacon_node/http_api/src/lib.rs`: Added `POST /eth/v1/builder/bids` route + `starts_with("v1/builder/bids")` reverse proxy filter; `SignedExecutionPayloadBid` added to imports
  - `common/eth2/src/lib.rs`: Added `post_builder_bids<E>` client method; `SignedExecutionPayloadBid` added to imports
- 181/181 http_api tests pass

### 2026-02-18 — execution payload envelope submission endpoint
- **Endpoint**: `POST /eth/v1/beacon/execution_payload_envelope`
- **What it does**: External builders submit a `SignedExecutionPayloadEnvelope` via HTTP. The BN runs gossip verification (block root known in fork choice, not finalized, slot match, builder index match, block_hash match, signature valid), gossips on the `execution_payload` P2P topic, then runs the full state transition asynchronously (notifies EL via `newPayload`, applies `process_execution_payload`, updates fork choice payload-revealed status).
- **Error handling**: Stale envelopes (`PriorToFinalization`) → 200 OK; all other errors → 400.
- **Fork guard**: Returns 400 "Gloas is not scheduled" if called pre-Gloas.
- **Design choice**: Gossip before process (so other nodes see it immediately), process after (async state transition).
- **Files changed**: 2 files
  - `beacon_node/http_api/src/lib.rs`: Added `POST /eth/v1/beacon/execution_payload_envelope` route using `spawn_async_with_rejection` (needed for async `process_payload_envelope`); `SignedExecutionPayloadEnvelope` added to imports
  - `common/eth2/src/lib.rs`: Added `post_beacon_execution_payload_envelope<E>` client method; `SignedExecutionPayloadEnvelope` added to imports
- 181/181 http_api tests pass

### 2026-02-18 — envelope DB persistence and retrieval endpoint
- **What**: Envelopes are now persisted to disk and retrievable via a new GET endpoint. Previously, envelopes were only cached in memory (state_cache) and lost on restart.
- **Store layer**:
  - New `DBColumn::BeaconEnvelope` (`"bev"`) with 32-byte keys (block root)
  - New `StoreOp::PutPayloadEnvelope` and `StoreOp::DeletePayloadEnvelope`
  - `StoreItem` impl for `SignedExecutionPayloadEnvelope` (SSZ serialization)
  - `get_payload_envelope()` and `payload_envelope_exists()` methods on `HotColdDB`
  - Envelopes stored in `hot_db` (not blobs_db — small size, keyed by block root like ExecPayload)
  - Finalization pruning: envelopes deleted alongside execution payloads during `try_prune_execution_payloads`
- **Beacon chain persistence**: Envelopes stored after successful processing in both `process_payload_envelope` (gossip path) and `process_self_build_envelope` (local self-build path)
- **New endpoint**: `GET /eth/v1/beacon/execution_payload_envelope/{block_id}` — resolves block_id via `BlockId::root()`, returns the stored envelope with execution_optimistic/finalized metadata and Gloas version header
- **Client method**: `get_beacon_execution_payload_envelope(block_id)` added to eth2 crate
- **Files changed**: 7 files (6 modified + 1 new)
  - `beacon_node/store/src/lib.rs`: DBColumn + StoreOp variants
  - `beacon_node/store/src/impls/execution_payload_envelope.rs` (new): StoreItem impl
  - `beacon_node/store/src/impls.rs`: module registration
  - `beacon_node/store/src/hot_cold_store.rs`: convert_to_kv_batch handlers, cache match arms, get/exists methods, pruning
  - `beacon_node/beacon_chain/src/beacon_chain.rs`: persist in both envelope processing paths, expose get_payload_envelope
  - `beacon_node/http_api/src/lib.rs`: GET endpoint + route wiring
  - `common/eth2/src/lib.rs`: client method
- 24/24 store tests pass, 136/136 EF tests pass, 181/181 http_api tests pass

### 2026-02-24 — Gloas HTTP API integration tests
- **What**: Added 5 integration tests in `fork_tests.rs` covering Gloas-specific endpoints that previously had zero test coverage:
  1. `ptc_duties_rejected_before_gloas_scheduled` — PTC duties returns 400 "Gloas is not scheduled" when fork not configured
  2. `ptc_duties_returns_duties_after_gloas` — PTC duties returns valid duties with correct epoch/validator bounds after Gloas activation
  3. `ptc_duties_rejects_future_epoch` — PTC duties rejects epoch too far in the future (>current+1)
  4. `get_execution_payload_envelope_not_found` — Envelope GET returns None/404 for non-existent block root
  5. `bid_submission_rejected_before_gloas` — Bid submission returns 400 "Gloas is not scheduled" pre-fork
- **Test infrastructure**: Reuses existing `InteractiveTester<MinimalEthSpec>` pattern; added `gloas_spec()` helper that enables all forks through Gloas
- **Files changed**: 1 file (`beacon_node/http_api/tests/fork_tests.rs`)
- 5/5 new tests pass, 186/186 total http_api tests pass

### 2026-02-24 — SSZ response support for 6 endpoints (#8892)
- **What**: Added `application/octet-stream` (SSZ) response support to 6 HTTP API endpoints per beacon-APIs spec v4.0.0+:
  - `GET /eth/v1/beacon/states/{state_id}/pending_deposits`
  - `GET /eth/v1/beacon/states/{state_id}/pending_partial_withdrawals`
  - `GET /eth/v1/beacon/states/{state_id}/pending_consolidations`
  - `GET /eth/v1/validator/attestation_data`
  - `GET /eth/v2/validator/aggregate_attestation`
  - `POST /eth/v1/beacon/states/{state_id}/validator_identities`
- **Pattern**: Each endpoint now checks the `Accept` header; when `application/octet-stream` is requested, the data is SSZ-encoded directly (bypassing JSON wrapper) with `Content-Type: application/octet-stream` and `Eth-Consensus-Version` headers
- **Also**: Added `Encode`/`Decode` derives to `ValidatorIdentityData` API type for SSZ support
- **Files changed**: 3 files
  - `beacon_node/http_api/src/lib.rs`: Accept header + SSZ branch for 5 endpoints
  - `beacon_node/http_api/src/aggregate_attestation.rs`: Accept header + SSZ branch
  - `common/eth2/src/types.rs`: SSZ derives on `ValidatorIdentityData`
- 212/212 http_api tests pass, 34/34 eth2 tests pass, full workspace clippy clean

### 2026-02-28 — GET payload attestations pool endpoint
- **Endpoint**: `GET /eth/v1/beacon/pool/payload_attestations?slot`
- **What it does**: Returns all payload attestations currently in the pool, optionally filtered by slot. This complements the existing `POST /eth/v1/beacon/pool/payload_attestations` endpoint which accepts `PayloadAttestationMessage` submissions.
- **Why**: Part of beacon-APIs spec (PR #552). Allows external tools, validators, and monitoring systems to query the current payload attestation pool state — useful for debugging PTC behavior and payload timeliness.
- **Response**: `{ "data": [PayloadAttestation, ...] }` — returns aggregated `PayloadAttestation` objects (with aggregation_bits, data, and signature).
- **Query parameters**: `slot` (optional) — when provided, only returns attestations targeting that slot.
- **Files changed**: 4 files
  - `beacon_node/beacon_chain/src/beacon_chain.rs`: Added `get_all_payload_attestations(slot_filter)` method
  - `beacon_node/http_api/src/lib.rs`: Added GET handler + route wiring
  - `common/eth2/src/lib.rs`: Added `get_beacon_pool_payload_attestations` client method + `PayloadAttestation` import
  - `common/eth2/src/types.rs`: Added `PayloadAttestationPoolQuery` struct
- 226/226 http_api tests pass
