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

### Tasks
- [ ] Update block proposal flow for ePBS (proposer creates block with bid selection)
- [ ] Implement bid selection logic (choose best bid from builders)
- [ ] Update duty discovery for new gloas duties
- [ ] Handle the case where no bids are received

## Progress log

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
