# gloas implementation workstream

> tracking EIP-7732 (ePBS) implementation for vibehouse

## spec reference

- **Primary spec**: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/beacon-chain.md
- **EIP**: https://eips.ethereum.org/EIPS/eip-7732
- **Upstream PRs**: 
  - [#8806 - Gloas payload processing](https://github.com/sigp/lighthouse/pull/8806)
  - [#8815 - Proposer lookahead](https://github.com/sigp/lighthouse/pull/8815)

## key concepts learned

### builder registry

Gloas introduces a **separate builder registry** alongside the validator registry. Builders are not validators.

- **Builder type**: new `BuilderIndex` type (uint64)
- **Builder container**: tracks pubkey, execution address, balance, deposit epoch, withdrawable epoch
- **Builder deposits**: use withdrawal prefix `0x03` (different from validator `0x01`)
- **Builder state**: stored in `BeaconState.builders` list
- **Builder activation**: deposit immediately adds to registry (unlike validators which queue)
- **Builder recycling**: builder indices can be reused after exit + balance = 0

Key insight: builders and validators are completely separate entities, but they share some machinery (deposits, exits).

### two-phase block structure (ePBS core)

The block/payload split is the heart of ePBS:

**Phase 1: Proposer block** (signed beacon block)
- Contains `SignedExecutionPayloadBid` - builder's commitment to build
- NO execution payload, NO blob commitments, NO execution requests
- Proposer selects best bid from builders
- Block is valid without payload

**Phase 2: Builder payload** (signed execution payload envelope)
- Contains actual `ExecutionPayload` + `ExecutionRequests`
- Builder reveals payload after proposer commits to their bid
- Separate state transition function: `process_execution_payload()`

This means:
- Beacon block can be imported without payload
- Payload arrives separately (same slot, later)
- Fork choice works on beacon blocks, payload attestations track delivery

### execution payload bid

`ExecutionPayloadBid` is the builder's commitment:

```python
class ExecutionPayloadBid(Container):
    parent_block_hash: Hash32      # links to previous payload
    parent_block_root: Root        # links to beacon block
    block_hash: Hash32             # hash of the payload builder commits to
    prev_randao: Bytes32           # must match beacon state randao
    fee_recipient: ExecutionAddress # where builder payment goes
    gas_limit: uint64              # gas limit of the payload
    builder_index: BuilderIndex    # who made this bid
    slot: Slot                     # must match block slot
    value: Gwei                    # how much builder pays proposer
    execution_payment: Gwei        # payment to execution layer (deprecated?)
    blob_kzg_commitments: List[KZGCommitment, MAX_BLOB_COMMITMENTS_PER_BLOCK]
```

Bid validation:
- Builder must be active (`is_active_builder`)
- Builder must have balance to cover bid (`can_builder_cover_bid`)
- Bid signature must be valid
- Bid slot/parent must match block
- Special case: `BUILDER_INDEX_SELF_BUILD` (proposer builds own payload, value = 0)

### payload timeliness committee (PTC)

New committee type for attesting to payload presence:

- **Size**: 512 validators (`PTC_SIZE`)
- **Selection**: balance-weighted sampling from all committees in a slot
- **Purpose**: attest that payload was received in time
- **Message**: `PayloadAttestation` with `PayloadAttestationData`:
  - `beacon_block_root`: which beacon block
  - `slot`: which slot
  - `payload_present`: did payload arrive?
  - `blob_data_available`: was blob data available?

PTC attestations are aggregated and included in `BeaconBlockBody.payload_attestations` (max 4 per block).

### builder payments and withdrawals

Builder payments follow a quorum-based system:

1. **Bid commitment**: builder bids value X
2. **Pending payment**: stored in `builder_pending_payments` vector (indexed by slot)
3. **PTC attestations**: validators attest to payload delivery
4. **Weight accumulation**: each PTC attestation adds attester weight to payment
5. **Quorum check**: at epoch boundary, check if `weight >= quorum_threshold`
6. **Payment execution**: if quorum met, move to `builder_pending_withdrawals`
7. **Withdrawal processing**: included in next payload's withdrawals

Quorum threshold = 60% of total active balance per slot:
```python
per_slot_balance = get_total_active_balance(state) // SLOTS_PER_EPOCH
quorum = per_slot_balance * 6 // 10
```

This means: if ≥60% of stake attests payload present, builder gets paid.

### state transition changes

Major reordering of block processing:

**Old (pre-gloas)**:
1. process_block_header
2. process_execution_payload (EL validation here)
3. process_randao
4. process_eth1_data
5. process_operations
6. process_sync_aggregate

**New (gloas)**:
1. process_block_header
2. **process_withdrawals** (moved up, before bid)
3. **process_execution_payload_bid** (new)
4. process_randao
5. process_eth1_data
6. process_operations (now includes payload attestations)
7. process_sync_aggregate

Then separately (different state transition):
8. **process_execution_payload** (from SignedExecutionPayloadEnvelope)

### withdrawals changes

Withdrawals now include builder withdrawals:

Order:
1. Builder pending withdrawals (from paid bids)
2. Validator partial withdrawals
3. Builder sweep withdrawals (exited builders)
4. Validator sweep withdrawals

Withdrawals are deterministic based on beacon state. They're computed before processing the bid, and the execution payload MUST honor them.

`process_withdrawals()` is called before `process_execution_payload_bid()` because the bid affects balances.

Key: if parent block is empty (no payload), skip withdrawals.

### fork choice integration

Fork choice needs new handlers:

- `on_execution_bid`: handle bid gossip
- `on_payload_attestation`: handle PTC attestation gossip
- Payload presence tracking via `execution_payload_availability` bitvector

Attestation participation flags now check payload matching:
- For same-slot attestations: payload always matches
- For other attestations: check if `data.index` matches `execution_payload_availability[slot]`

### networking changes

New gossip topics:
- `/eth2/beacon_chain/req/execution_bid/1/ssz_snappy`
- `/eth2/beacon_chain/req/execution_payload_envelope/1/ssz_snappy`
- `/eth2/beacon_chain/req/payload_attestation/1/ssz_snappy`

Gossip validation rules needed for each.

### data availability changes (EIP-7916)

`DataColumnSidecar` changes:
- **Removed**: `signed_block_header` field
- **Removed**: `kzg_commitments_inclusion_proof` field

Instead: verify kzg_commitments hash against builder bid's `blob_kzg_commitments`.

## implementation status

### completed
- [x] Spec research and documentation (this file)

### in progress
- [ ] None

### todo
See plan.md for detailed checklist. High-level phases:
1. Types & Constants
2. State Transition
3. Fork Choice
4. P2P Networking
5. Beacon Chain Integration
6. Validator Client
7. REST API
8. Testing

## blockers

- **Rust toolchain**: host doesn't have cargo/rustc installed
- **Spec test runner**: need to understand lighthouse test infrastructure before implementing
- **Upstream tracking**: PR #8806 and #8815 are WIP, may change

## decisions

- Start with types and SSZ before state transitions
- Reference upstream PRs but don't copy blindly (they may be outdated)
- Write tests alongside implementation, not after
- Document every deviation from spec with a `// VIBEHOUSE:` comment

## notes

- Builders are NOT validators
- Proposers select bids, builders deliver payloads
- PTC attestations gate builder payments (60% quorum)
- Withdrawals happen before bid processing
- Fork choice operates on beacon blocks; payload delivery is tracked separately
- Builder indices are reusable after exit

## next steps

1. Set up CI with spec test runner baseline
2. Create type definitions in `consensus/types/src/`
3. Start state transition implementation in `consensus/state_processing/src/`
4. Track upstream PR #8806 for reference
5. Write unit tests for each new type
