# Gloas State Transition Implementation

## Phase 2 Status: Starting

### Reference
- Spec: https://github.com/ethereum/consensus-specs/tree/master/specs/gloas/beacon-chain.md
- Upstream PR: https://github.com/sigp/lighthouse/pull/8806

### Required State Transition Functions

#### Per-Block Processing

1. **`process_execution_payload_bid`**
   - Location: `consensus/state_processing/src/per_block_processing/process_operations.rs`
   - Validates and applies SignedExecutionPayloadBid to state
   - Checks:
     - Builder exists and is active
     - Signature valid
     - Bid commitment matches (block_hash, blob_kzg_commitments)
     - Builder has sufficient balance
   - Updates:
     - `state.latest_execution_payload_bid`
     - Builder pending payment (adds to builder_pending_payments)

2. **`process_payload_attestation`**
   - Location: `consensus/state_processing/src/per_block_processing/process_operations.rs`
   - Validates payload attestations from PTC members
   - Checks:
     - Attestation is for current slot
     - Aggregation bits correspond to valid PTC members
     - Signature valid from PTC committee
   - Updates:
     - Sets `execution_payload_availability[slot]` if quorum reached
     - Triggers builder payment if payload was revealed

3. **`process_withdrawals` modifications for ePBS**
   - Location: `consensus/state_processing/src/per_block_processing/` (existing file)
   - Changes:
     - Include builder withdrawals in the withdrawal queue
     - Process builder_pending_withdrawals alongside validator withdrawals

4. **Builder payment processing**
   - Triggered by payload attestations reaching quorum
   - Transfer bid value from builder balance to proposer
   - Clear builder_pending_payment for the slot

#### Per-Epoch Processing

1. **Builder registry updates**
   - Process builder deposits (similar to validator deposits)
   - Process builder withdrawals
   - Update builder balances

2. **Sweep builder pending withdrawals**
   - Similar to validator exit queue
   - Process MAX_BUILDERS_PER_WITHDRAWALS_SWEEP per epoch

3. **Clean up old pending payments**
   - Remove builder_pending_payments older than SLOTS_PER_HISTORICAL_ROOT

### Implementation Plan

**Week 1: Block Processing Core**
- [ ] Implement process_execution_payload_bid
- [ ] Write unit tests for bid validation
- [ ] Write unit tests for balance checks
- [ ] Write unit tests for signature verification

**Week 2: Payload Attestations**
- [ ] Implement process_payload_attestation
- [ ] Implement PTC committee calculation
- [ ] Write tests for attestation aggregation
- [ ] Write tests for quorum threshold

**Week 3: Builder Payments & Withdrawals**
- [ ] Implement builder payment trigger logic
- [ ] Modify process_withdrawals for builder withdrawals
- [ ] Write integration tests for payment flow
- [ ] Write tests for withdrawal queue

**Week 4: Epoch Processing**
- [ ] Implement epoch-level builder processing
- [ ] Implement pending payment cleanup
- [ ] Write comprehensive integration tests
- [ ] Test fork transition (Fulu → Gloas)

### Testing Strategy

Each function needs:
1. **Unit tests**: isolated function behavior
2. **Integration tests**: full block/epoch processing
3. **Edge case tests**:
   - Missing payload (builder doesn't reveal)
   - Insufficient builder balance
   - Invalid signatures
   - Quorum not reached
   - Self-build (builder_index == BUILDER_INDEX_SELF_BUILD)
4. **Fork transition tests**: Fulu → Gloas boundary

### Blockers

None currently. Types and constants are complete.

### Next Actions

1. Study consensus spec process_execution_payload_bid
2. Create skeleton functions in state_processing
3. Implement validation logic step by step
4. Test as we go (no ignored tests!)
