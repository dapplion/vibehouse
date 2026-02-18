# Validator Client (Phase 6)

## Objective
Update the validator client for ePBS: block proposal flow with bid selection, payload attestation duty, duty discovery.

## Status: IN PROGRESS

### Done
- ✅ Self-build envelope signing via VC (DOMAIN_BEACON_BUILDER, SignableMessage::ExecutionPayloadEnvelope)
- ✅ BN REST API for PTC duties: `POST /eth/v1/validator/duties/ptc/{epoch}`
- ✅ BN REST API for payload attestation data: `GET /eth/v1/validator/payload_attestation_data?slot`
- ✅ BN REST API for payload attestation submission: `POST /eth/v1/beacon/pool/payload_attestations`
- ✅ PayloadAttestationData signing via SignableMessage + ValidatorStore trait
- ✅ BN API client methods (eth2 crate) for PTC duties, attestation data, submission
- ✅ PayloadAttestationService: polls PTC duties, signs at 3/4 slot, publishes
- ✅ Service wired into VC startup
- ✅ Fork guards: PTC service disabled pre-Gloas, BN PTC duties endpoint rejects pre-Gloas
- ✅ ExecutionBidPool + bid selection in block production (BN side)
- ✅ PTC duty discovery integrated into DutiesService (proactive polling, notifier visibility)

### Tasks
- [ ] VC-side awareness of external builder blocks (currently VC always expects self-build envelope)

## Progress log

### 2026-02-18 — PTC duty discovery integrated into DutiesService
- **What**: Moved PTC duty polling from PayloadAttestationService's ad-hoc private cache into the centralized DutiesService, following the sync committee duty pattern.
- **New module**: `validator_services/src/ptc.rs` — `PtcDutiesMap` (epoch→duties map with `duties_for_slot()`, `duty_count()`, `prune()`), `poll_ptc_duties()` (proactive fetch for current + next epoch, Gloas fork guard, old epoch pruning)
- **DutiesService changes**: new `ptc_duties: PtcDutiesMap` field, `ptc_attester_count(epoch)` method, fifth polling task (`duties_service_ptc`) in `start_update_service` — runs every slot alongside attester/proposer/sync committee polling
- **PayloadAttestationService refactored**: takes `duties_service` reference via builder, reads duties from `DutiesService.ptc_duties.duties_for_slot()` instead of managing its own `DutiesCache`. Removed private `DutiesCache`, `get_duties_for_epoch()`, and `tokio::sync::RwLock` dependency.
- **Notifier**: now displays `ptc_attesters` count alongside `current_epoch_proposers` and `active_validators` in the "All validators active" / "Some validators active" log lines
- **Tests**: 136/136 EF tests (fake_crypto), 1302/1302 workspace tests pass, clippy clean

### 2026-02-18 — bid selection in block production (BN side)
- **What**: Added `ExecutionBidPool` and bid selection logic to block production. When external builder bids are available for the proposal slot, the BN selects the highest-value bid and includes it in the block instead of self-building. Falls back to self-build when no external bids exist.
- **New data structure**: `ExecutionBidPool` — stores full verified `SignedExecutionPayloadBid` objects per (slot, builder_index). Auto-prunes old slots. One bid per builder per slot (first valid bid wins, equivocation handled at gossip layer).
- **Bid insertion points**: Bids are added to the pool in `apply_execution_bid_to_fork_choice()`, which is called from both the gossip handler and the HTTP `POST /eth/v1/builder/bids` endpoint.
- **Bid selection flow**:
  1. `produce_partial_beacon_block()` calls `get_best_execution_bid(slot)` before starting EL payload fetch
  2. If an external bid exists: skip `get_execution_payload()` entirely (no EL call needed — builder provides payload via envelope)
  3. Store `selected_external_bid` in `PartialBeaconBlock`
  4. `complete_partial_beacon_block()` Gloas branch: if external bid present, use it directly; else fall back to self-build (unchanged behavior)
- **External bid path**: No self-build envelope is constructed. The builder reveals the execution payload after the block is published, via a separate `SignedExecutionPayloadEnvelope` gossip/API submission.
- **Self-build fallback**: Identical to previous behavior — calls EL `engine_getPayload`, builds `ExecutionPayloadBid` with `builder_index = BUILDER_INDEX_SELF_BUILD`, constructs self-build envelope for VC signing.
- **Files changed**: 4 files (3 modified + 1 new)
  - `beacon_node/beacon_chain/src/execution_bid_pool.rs` (new): `ExecutionBidPool` struct with `insert()`, `get_best_bid()`, `prune()`, 4 unit tests
  - `beacon_node/beacon_chain/src/lib.rs`: registered `execution_bid_pool` module
  - `beacon_node/beacon_chain/src/beacon_chain.rs`: added `execution_bid_pool` field to `BeaconChain`, `get_best_execution_bid()` method, `selected_external_bid` field on `PartialBeaconBlock`, bid selection in `produce_partial_beacon_block()`, external bid path in `complete_partial_beacon_block()` Gloas branch
  - `beacon_node/beacon_chain/src/builder.rs`: initialized `execution_bid_pool` in `BeaconChainBuilder`
- 136/136 EF tests (fake_crypto), 1302/1302 workspace tests pass, clippy clean

### 2026-02-17 — fork guards for payload attestation service and PTC endpoint
- **Problem**: PayloadAttestationService started unconditionally at VC startup, polling for PTC duties every slot even before Gloas fork. BN's PTC duties endpoint (`POST /eth/v1/validator/duties/ptc/{epoch}`) had no check for Gloas being scheduled.
- **Fix 1**: PayloadAttestationService now checks `is_gloas_scheduled()` at startup — if Gloas isn't scheduled, the service logs a message and returns without spawning. If scheduled but not yet activated, the main loop skips duty polling via `gloas_fork_activated()` check (matches sync_committee_service pattern with `altair_fork_activated`).
- **Fix 2**: PTC duties endpoint now returns 400 "Gloas is not scheduled" when Gloas fork epoch isn't configured (matches custody endpoint pattern with `is_fulu_scheduled`).
- **Fix 3**: Added `gloas_fork_epoch` to `PayloadAttestationServiceBuilder` and `Inner` struct, passed from spec at builder construction.
- 78/78 EF tests pass, 136/136 fake_crypto pass (unchanged)

### 2026-02-17 — VC payload attestation service
- Added `PayloadAttestationData` variant to `SignableMessage` enum with signing_root + Web3Signer error
- Added `sign_payload_attestation` to `ValidatorStore` trait and `LighthouseValidatorStore` impl (Domain::PtcAttester)
- Added 3 BN API client methods to `common/eth2/src/lib.rs` (get_validator_payload_attestation_data, post_validator_duties_ptc, post_beacon_pool_payload_attestations)
- Created `payload_attestation_service.rs` in validator_services: builder pattern, epoch-cached duty fetching, sign+submit at 3/4 slot
- Wired into `ProductionValidatorClient` startup alongside existing services
- All 78 EF tests + 136 fake_crypto tests pass

### 2026-02-17 — PTC duty and payload attestation REST API endpoints (BN side)
- Added three new BN endpoints needed by the VC to produce payload attestations
- See [beacon-chain-integration.md](beacon-chain-integration.md) for implementation details
- Next step: implement the VC-side `payload_attestation_service` that uses these endpoints
