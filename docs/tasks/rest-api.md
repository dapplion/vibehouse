# REST API (Phase 7)

## Objective
Add ePBS-specific REST API endpoints for block submission, bid submission, payload attestation, and SSE events.

## Status: IN PROGRESS

### Done
- [x] SSE events for ePBS: `execution_bid`, `execution_payload`, `payload_attestation`
- [x] `GET /eth/v1/beacon/states/{state_id}/proposer_lookahead` — Fulu/Gloas only, returns `proposer_lookahead` vector

### Tasks
- [ ] Add `/eth/v1/beacon/blinded_blocks` updates for ePBS
- [ ] Add execution bid submission endpoint
- [ ] Update block retrieval endpoints to handle two-phase blocks

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
