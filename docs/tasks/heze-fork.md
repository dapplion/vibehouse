# Heze Fork Implementation (EIP-7805: FOCIL)

## Objective

Implement the Heze consensus-layer fork, which adds Fork-Choice Enforced Inclusion Lists (FOCIL) per EIP-7805. Heze is the fork after Gloas.

## Reference

- Spec: https://github.com/ethereum/consensus-specs/tree/master/specs/heze
- EIP: https://eips.ethereum.org/EIPS/eip-7805

## Spec Summary

Heze adds inclusion lists — a mechanism for committees of 16 validators per slot to submit transaction inclusion requirements that builders must satisfy. Key changes:

### New Types
- `InclusionList`: slot, validator_index, inclusion_list_committee_root, transactions
- `SignedInclusionList`: message + signature
- `InclusionListStore`: local tracking of inclusion lists and equivocators

### Modified Types
- `ExecutionPayloadBid`: adds `inclusion_list_bits: Bitvector[INCLUSION_LIST_COMMITTEE_SIZE]`
- `BeaconState`: `latest_execution_payload_bid` uses new ExecutionPayloadBid with inclusion_list_bits

### New Constants
- `DOMAIN_INCLUSION_LIST_COMMITTEE = 0x0E000000`
- `INCLUSION_LIST_COMMITTEE_SIZE = 16`

### Fork Config
- `HEZE_FORK_VERSION = 0x08000000`
- `HEZE_FORK_EPOCH = TBD`

### New Functions
- `get_inclusion_list_committee(state, slot)` — 16 validators from slot committees
- `is_valid_inclusion_list_signature(state, signed_il)` — BLS signature check
- `process_inclusion_list(store, il, is_before_cutoff)` — equivocation detection + storage
- `get_inclusion_list_transactions(store, state, slot)` — deduplicated tx list
- `get_inclusion_list_bits(store, state, slot)` — bitvector of valid submissions
- `is_inclusion_list_bits_inclusive(store, state, slot, bits)` — superset check

### Fork Choice Changes
- New Store field: `payload_inclusion_list_satisfaction: Dict[Root, bool]`
- `should_extend_payload()` checks inclusion list satisfaction
- `on_execution_payload()` records satisfaction
- `on_inclusion_list()` handler
- New timing: view freeze cutoff at 75% of slot

### P2P
- New gossip topic: `inclusion_list`
- New req/resp: `InclusionListByCommitteeIndices/1`
- `MAX_REQUEST_INCLUSION_LIST = 16`
- `MAX_BYTES_PER_INCLUSION_LIST = 8192`

### Validator
- Inclusion list committee duty discovery
- IL construction + broadcast at ~67% of slot
- Bid validation: `inclusion_list_bits` must be inclusive
- PayloadAttributes gains `inclusion_list_transactions`

### Builder
- Set `bid.inclusion_list_bits` from local IL store

## Phases

| Phase | Description | Status |
|-------|-------------|--------|
| 1. Types & Constants | ForkName, ChainSpec, EthSpec, new types, superstruct variants | DONE |
| 2. State Transitions | Fork upgrade, inclusion list committee computation | DONE |
| 3. Fork Choice | IL satisfaction tracking, should_extend_payload changes | NOT STARTED |
| 4. P2P Networking | Gossip topic, req/resp protocol, validation | NOT STARTED |
| 5. Beacon Chain Integration | IL store, builder bid validation | NOT STARTED |
| 6. Validator Client | IL committee duties, IL construction, bid validation | NOT STARTED |
| 7. REST API | IL endpoints | NOT STARTED |

## Progress Log

### Phase 1: Types & Constants (run 3346)

Starting Heze fork implementation. Adding ForkName::Heze variant and propagating through the codebase.

**Completed:** ForkName::Heze added and propagated through entire codebase (40 files, +1300/-213 lines).

Changes:
- **ForkName**: Added `Heze` variant after `Gloas`, updated `list_all()`, `from_str`, `fork_epoch`, all match arms
- **ChainSpec**: `heze_fork_epoch`, `heze_fork_version` (0x08000000)
- **BeaconBlock/Body**: Heze superstruct variants (ePBS: signed_execution_payload_bid + payload_attestations, no execution_payload)
- **BeaconState**: Heze variant, `upgrade_to_heze()` state transition in state_processing
- **ExecutionPayload/Header**: Heze variants (same fields as Gloas — no EL changes in Heze)
- **BuilderBid**: Heze variant with ExecutionPayloadHeaderHeze
- **FullPayload/BlindedPayload**: Heze variants (BlindedPayload shares ExecutionPayloadHeaderGloas with Gloas via `only(Gloas, Heze)`)
- **LightClientHeader/Bootstrap/Update/FinalityUpdate/OptimisticUpdate**: Heze variants (share ExecutionPayloadHeaderGloas with Gloas)
- **SignedBeaconBlock**: Heze variant, SSZ decode, blinded↔full conversion
- **ExecutionLayer**: engine_api, json_structures, new_payload_request, mock_builder — Heze arms mirroring Gloas
- **Network**: RPC codec, protocol, pubsub — Heze arms mirroring Gloas
- **Validator**: web3signer Heze handling
- **EF tests**: fork upgrade, transition, merkle proof test runners — Heze support
- **Tests**: 1088/1088 types tests pass, 1033/1033 state_processing tests pass

### Phase 2: State Transitions — Part 1 (run 3347)

Adding core FOCIL types and helpers from the Heze spec (EIP-7805).

**Completed:**

1. **InclusionList type** (`consensus/types/src/inclusion_list.rs`): New container with `slot`, `validator_index`, `inclusion_list_committee_root`, `transactions` (bounded by `MaxTransactionsPerPayload`). SSZ, TreeHash, serde all derived. 4 tests.

2. **SignedInclusionList type** (`consensus/types/src/signed_inclusion_list.rs`): Signed wrapper (`message: InclusionList`, `signature: BLSSignature`). `SignedRoot` impl on `InclusionList`. 5 tests.

3. **get_inclusion_list_committee** (`consensus/state_processing/src/per_block_processing/heze.rs`): Returns `INCLUSION_LIST_COMMITTEE_SIZE` (16) validator indices by concatenating all beacon committees for the slot and cycling through via modulo. Matches spec exactly. 5 tests.

4. **is_valid_inclusion_list_signature** (same file): Validates signed inclusion list using `DOMAIN_INCLUSION_LIST_COMMITTEE` domain. Follows same pattern as bid signature verification.

5. **InclusionListInvalid error variant**: Added to `BlockProcessingError` for inclusion list validation failures.

Tests: 1101/1101 types tests pass (+13 new), 1038/1038 state_processing tests pass (+5 new)

### Phase 2: State Transitions — Part 2 (run 3348)

Adding `inclusion_list_bits` to ExecutionPayloadBid and InclusionListCommitteeSize to EthSpec.

**Design decision:** Rather than splitting `ExecutionPayloadBid` into separate Gloas/Heze types (which would require updating ~140 getter call sites and ~100 construction sites), we added `inclusion_list_bits: BitVector<E::InclusionListCommitteeSize>` directly to the existing `ExecutionPayloadBid<E>`. Since Gloas is our custom fork, adding the field there (initialized to all-zeros) is clean and avoids massive downstream churn.

**Completed:**

1. **InclusionListCommitteeSize** (`consensus/types/src/eth_spec.rs`): Added to EthSpec trait, set to `U16` for all three specs (Mainnet, Minimal, Gnosis).

2. **ExecutionPayloadBid** (`consensus/types/src/execution_payload_bid.rs`): Added `inclusion_list_bits: BitVector<E::InclusionListCommitteeSize>` field. Default is all-zeros. SSZ encoding changes for both Gloas and Heze bids.

3. **Construction sites fixed**: beacon_chain.rs (2 self-build bid sites), lcli/submit_builder_bid.rs, execution_bid_pool.rs (3 test helpers), execution_payload_bid.rs (1 test).

4. **upgrade_to_heze**: No changes needed — the bid is cloned as-is, preserving `inclusion_list_bits` at all-zeros from Gloas.

Tests: 1101/1101 types tests pass, 1038/1038 state_processing tests pass. Clean clippy.

**Phase 2 complete.** All FOCIL types, inclusion list committee computation, signature validation, and inclusion_list_bits in ExecutionPayloadBid are implemented.
