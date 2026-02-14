# 2026-02-14 10:34 - Phase 4: P2P Networking - Gossip Validation

## ethvibes session: gloas 24/7 🎵

### Mission
Continue Phase 4 implementation: Add gossip validation for ePBS messages

### Context
- **Phase 1-3**: ✅ COMPLETE (types, state transitions, fork choice)
- **Phase 4 (P2P)**: 🚧 IN PROGRESS (1/6 complete - topics added)

### Work Plan
Phase 4 Item 2: Gossip validation

Break down:
1. ✅ Gossip topics added (previous session)
2. 🚧 Gossip validation (this session)
   - Create verification error types
   - Implement `verify_execution_bid_for_gossip()`
   - Implement `verify_payload_attestation_for_gossip()`
   - Add signature verification helpers
3. ⏸️ Beacon processor integration
4. ⏸️ Equivocation detection caches
5. ⏸️ Peer scoring
6. ⏸️ Tests

### Session: Gossip Validation Infrastructure

#### What Was Done

**1. Created `gloas_verification.rs` module** (362 lines)

New file: `beacon_node/beacon_chain/src/gloas_verification.rs`

**Error types defined:**
- `ExecutionBidError` - 12 variants covering all bid validation failures
  - Slot timing (future/past)
  - Builder validation (unknown, inactive, insufficient balance)
  - Equivocation detection
  - Signature validation
  - Parent root validation
- `PayloadAttestationError` - 13 variants covering all attestation validation failures
  - Slot timing
  - Block root validation
  - PTC membership validation
  - Equivocation detection
  - Aggregation bits validation
  - Signature validation

**Verified types:**
- `VerifiedExecutionBid<T: BeaconChainTypes>` - Wrapper for validated bids
- `VerifiedPayloadAttestation<T: BeaconChainTypes>` - Wrapper for validated attestations

**Validation functions (stubbed):**
```rust
impl<T: BeaconChainTypes> BeaconChain<T> {
    pub fn verify_execution_bid_for_gossip(
        &self,
        bid: SignedExecutionPayloadBid<T::EthSpec>,
    ) -> Result<VerifiedExecutionBid<T>, ExecutionBidError>

    pub fn verify_payload_attestation_for_gossip(
        &self,
        attestation: PayloadAttestation<T::EthSpec>,
    ) -> Result<VerifiedPayloadAttestation<T>, PayloadAttestationError>
}
```

**Validation checks implemented:**
1. ✅ Slot timing validation (gossip clock disparity)
2. ✅ Aggregation bits validation (non-empty check)
3. 🚧 Builder registry validation (TODOs - needs state accessors)
4. 🚧 Equivocation detection (TODOs - needs observed caches)
5. 🚧 Signature verification (TODOs - signature_sets integration)
6. 🚧 PTC committee calculation (TODOs - needs state methods)

**2. Added signature_sets functions**

File: `consensus/state_processing/src/per_block_processing/signature_sets.rs`

Added 2 new signature set constructors:

```rust
pub fn execution_payload_bid_signature_set<'a, E, F>(...) -> Result<SignatureSet<'a>>
```
- Validates builder bid signature
- Uses `DOMAIN_BEACON_BUILDER` domain
- Looks up builder pubkey from builder registry

```rust
pub fn payload_attestation_signature_set<'a, E, F>(...) -> Result<SignatureSet<'a>>
```
- Validates PTC attestation aggregate signature
- Uses `DOMAIN_PTC_ATTESTER` domain
- Collects pubkeys from attesting validator indices

**3. Module registration**

Updated `beacon_node/beacon_chain/src/lib.rs`:
- Added `pub mod gloas_verification;`

#### Implementation Strategy

Following existing Lighthouse patterns:
- **Error types** mirror sync_committee_verification.rs structure
- **Verified wrappers** follow attestation_verification.rs pattern
- **Signature sets** match existing signature_sets.rs style
- **Early rejection** for invalid messages (peer scoring context in error docs)

#### TODOs (Next Iteration)

The validation functions are stubbed with TODOs for:

1. **State accessors** (needs gloas BeaconState methods):
   - `state.builders()` - access builder registry
   - `state.get_ptc_committee(slot)` - compute PTC for slot
   - Builder balance queries

2. **Observed caches** (equivocation detection):
   - `ObservedExecutionBids` - track (builder_index, slot) → bid_root
   - `ObservedPayloadAttestations` - track (validator_index, slot, block_root) → payload_present

3. **Fork choice integration**:
   - Parent root validation against fork choice head
   - Block root existence checks

4. **Signature verification wiring**:
   - Call `execution_payload_bid_signature_set()`
   - Call `payload_attestation_signature_set()`
   - Batch verification where applicable

#### Files Modified

- ✅ `beacon_node/beacon_chain/src/gloas_verification.rs` (NEW - 362 lines)
- ✅ `beacon_node/beacon_chain/src/lib.rs` (1 line added)
- ✅ `consensus/state_processing/src/per_block_processing/signature_sets.rs` (86 lines added)

#### Compilation Status

Running `cargo check -p beacon_chain` to verify:
- Type definitions compile
- Signature sets integrate cleanly
- Error types are well-formed

(Build in progress...)

### Next Steps

When build completes and passes:

1. **Implement observed caches**:
   - Create `beacon_node/beacon_chain/src/observed_execution_bids.rs`
   - Create `beacon_node/beacon_chain/src/observed_payload_attestations.rs`
   - Follow `observed_attesters.rs` pattern

2. **Wire up signature verification**:
   - Complete TODOs in verify_execution_bid_for_gossip
   - Complete TODOs in verify_payload_attestation_for_gossip

3. **Beacon processor integration** (Phase 4 Item 3):
   - Add handlers in `gossip_methods.rs`
   - Call verification functions
   - Propagate valid messages
   - Import to fork choice

4. **Tests**:
   - Unit tests for error cases
   - Integration tests for gossip flow

### Status

**Phase 4 Progress**: 1.5/6 items complete
- ✅ Gossip topics added
- 🚧 Gossip validation (infrastructure complete, wiring needed)
- ⏸️ Beacon processor integration
- ⏸️ Equivocation detection caches
- ⏸️ Peer scoring
- ⏸️ Tests

**Vibes**: Solid foundation. The verification module structure is clean and follows Lighthouse patterns. TODOs are well-documented. Next session will complete the wiring.

🎵 ethvibes - building the ePBS validation stack
