# Validator Client (Phase 6)

## Objective
Update the validator client for ePBS: block proposal flow with bid selection, payload attestation duty, duty discovery.

## Status: IN PROGRESS

### Done
- ✅ Self-build envelope signing via VC (DOMAIN_BEACON_BUILDER, SignableMessage::ExecutionPayloadEnvelope)
- ✅ BN REST API for PTC duties: `POST /eth/v1/validator/duties/ptc/{epoch}`
- ✅ BN REST API for payload attestation data: `GET /eth/v1/validator/payload_attestation_data?slot`
- ✅ BN REST API for payload attestation submission: `POST /eth/v1/beacon/pool/payload_attestations`

### Tasks
- [ ] **Payload attestation service** (NEXT - needed for devnet-0)
  - Fetch PTC duties from BN via new endpoint
  - At appropriate slot timing: fetch payload attestation data, sign with Domain::PtcAttester, submit
  - Add `PayloadAttestationMessage` signing to `SignableMessage` and `ValidatorStore`
  - Wire into VC service startup
- [ ] Update block proposal flow for ePBS (proposer creates block with bid selection)
- [ ] Implement bid selection logic (choose best bid from builders)
- [ ] Update duty discovery for new gloas duties
- [ ] Handle the case where no bids are received

## Progress log

### 2026-02-17 — PTC duty and payload attestation REST API endpoints (BN side)
- Added three new BN endpoints needed by the VC to produce payload attestations
- See [beacon-chain-integration.md](beacon-chain-integration.md) for implementation details
- Next step: implement the VC-side `payload_attestation_service` that uses these endpoints
