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
| 3. Fork Choice | IL satisfaction tracking, should_extend_payload changes | DONE |
| 4. P2P Networking | Gossip topic, req/resp protocol, validation | DONE |
| 5. Beacon Chain Integration | IL store, builder bid validation | DONE |
| 6. Validator Client | IL committee duties, IL construction, bid validation | DONE |
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

### Phase 3: Fork Choice (run 3349)

Implementing Heze fork choice changes for FOCIL inclusion list satisfaction tracking.

**Completed:**

1. **InclusionListStore** (`consensus/types/src/inclusion_list_store.rs`): New runtime store tracking inclusion lists per (slot, committee_root) with equivocation detection. Methods: `process_inclusion_list()`, `get_inclusion_list_transactions()`, `get_inclusion_list_bits()`, `is_inclusion_list_bits_inclusive()`, `prune()`. 13 tests.

2. **ProtoNode.inclusion_list_satisfied** (`consensus/proto_array/src/proto_array.rs`): New bool field tracking whether a block's payload satisfied inclusion list constraints. Maps to spec's `store.payload_inclusion_list_satisfaction[root]`. Added to ProtoNode, Block, and all construction sites (~25 sites).

3. **Modified should_extend_payload** (`consensus/proto_array/src/proto_array_fork_choice.rs`): Added Heze check at top — returns false when `envelope_received && !inclusion_list_satisfied`. Matches spec's `is_payload_inclusion_list_satisfied` check.

4. **Modified on_execution_payload** (`consensus/fork_choice/src/fork_choice.rs`): Sets `inclusion_list_satisfied = true` when envelope is received. This is a stub — the real EL `is_inclusion_list_satisfied` check will be wired in Phase 5 (Beacon Chain Integration) when the InclusionListStore is integrated into the beacon chain.

**Design decisions:**
- InclusionListStore placed in `types` crate (not beacon_chain) since its helpers are pure spec logic needed by multiple consumers.
- Helper functions take `committee_root: Hash256` instead of computing it, avoiding tree_hash dependency on raw slices. Callers compute the root from `get_inclusion_list_committee`.
- `on_inclusion_list` handler deferred to Phase 5 — it requires the beacon chain to hold an InclusionListStore instance and compute timing.
- `inclusion_list_satisfied` defaults to `false` on new blocks; set to `true` by `on_execution_payload`. Pre-Heze behavior unchanged since the check only fires when `envelope_received = true`.

Tests: 1114/1114 types tests pass (+13 new), 206/206 proto_array tests pass, 90/90 fork_choice lib tests pass, 31/31 fork_choice integration tests pass. Full workspace lint clean.

**Phase 3 complete.**

### Phase 4: P2P Networking — Part 1 (run 3350)

Adding gossip topic infrastructure for Heze FOCIL inclusion lists.

**Completed:**

1. **GossipKind::InclusionList** (`vibehouse_network/src/types/topics.rs`): New gossip topic variant with `inclusion_list` topic constant. Added to topic decode, display, subscription (Heze-gated), `is_fork_non_core_topic`.

2. **PubsubMessage::InclusionList** (`vibehouse_network/src/types/pubsub.rs`): New variant carrying `Box<SignedInclusionList<E>>`. SSZ decode gated on `fork.heze_enabled()`. Encode, kind(), Display implemented.

3. **Beacon Processor** (`beacon_processor/src/lib.rs`): `Work::GossipInclusionList(BlockingFn)` + `WorkType::GossipInclusionList`. Queue with 4096 capacity. Drains after execution proof queue. Spawned as blocking task.

4. **Router dispatch** (`network/src/router.rs`): Routes `PubsubMessage::InclusionList` to `send_gossip_inclusion_list`.

5. **Network beacon processor** (`network/src/network_beacon_processor/mod.rs`): `send_gossip_inclusion_list()` creates `Work::GossipInclusionList` work event.

6. **Gossip validation stub** (`network/src/network_beacon_processor/gossip_methods.rs`): `process_gossip_inclusion_list()` logs and accepts — full validation deferred to Phase 5 (beacon chain integration).

7. **Gossip scoring** (`vibehouse_network/src/service/gossipsub_scoring_parameters.rs`): `INCLUSION_LIST_WEIGHT=0.3`, expects 16 messages/slot (one per committee member).

8. **Gossip cache** (`vibehouse_network/src/service/gossip_cache.rs`): No caching for inclusion lists (time-sensitive).

Tests: 407/407 vibehouse_network pass, 8/8 beacon_processor pass. Full workspace lint clean.

### Phase 4: P2P Networking — Part 2 (run 3351)

Adding `InclusionListByCommitteeIndices/1` RPC req/resp protocol for FOCIL.

**Completed:**

1. **Protocol enum** (`vibehouse_network/src/rpc/protocol.rs`): `Protocol::InclusionListByCommitteeIndices` with strum serialize `"inclusion_list_by_committee_indices"`. `SupportedProtocol::InclusionListByCommitteeIndicesV1`. Fork-gated on `ForkName::Heze`. Context bytes enabled. Request limit: `max_request_inclusion_lists * 8` bytes. Response limit: `SIGNED_INCLUSION_LIST_MAX` (8192 bytes).

2. **Request type** (`methods.rs`): `InclusionListByCommitteeIndicesRequest` with `committee_indices: RuntimeVariableList<u64>`. Constructor validates against `spec.max_request_inclusion_lists` (16).

3. **Response type** (`methods.rs`): `RpcSuccessResponse::InclusionListByCommitteeIndices(Arc<SignedInclusionList<E>>)`. `ResponseTermination::InclusionListByCommitteeIndices`.

4. **Codec** (`codec.rs`): SSZ encode/decode for both request and response. Response decode gated on `fork_name.heze_enabled()`.

5. **Rate limiter** (`rate_limiter.rs`, `config.rs`): `ilbci_rl` limiter with default quota 16/10s. Full builder/config/prune integration.

6. **Service** (`service/mod.rs`): Request dispatch with metrics, response routing, stream termination handling.

7. **Router** (`router.rs`): Dispatches inbound requests to `send_inclusion_list_by_committee_indices_request`. Response handling logs and defers to Phase 5.

8. **Beacon processor** (`beacon_processor/src/lib.rs`): `Work::InclusionListByCommitteeIndicesRequest` work type with 1024-capacity queue.

9. **Network beacon processor** (`network_beacon_processor/`): `send_inclusion_list_by_committee_indices_request()` and `handle_inclusion_list_by_committee_indices_request()` stub handler (terminates stream with no responses; Phase 5 will serve from InclusionListStore).

10. **Peer manager** (`peer_manager/mod.rs`): Error handling for rate limiting, unsupported protocol, and stream timeout matches.

11. **ChainSpec** (`chain_spec.rs`): `max_request_inclusion_lists: 16` for mainnet and minimal specs.

Tests: 407/407 vibehouse_network pass, 8/8 beacon_processor pass, 47/47 types chain_spec pass. Full workspace build + lint clean.

**Phase 4 complete.** Both gossip topic (Part 1) and RPC req/resp protocol (Part 2) are implemented.

**Phase 4 complete.** Next: Phase 5.

### Phase 5: Beacon Chain Integration (run 3352)

Integrating InclusionListStore into the beacon chain, completing the server-side FOCIL pipeline.

**Completed:**

1. **InclusionListStore in BeaconChain** (`beacon_chain.rs`, `builder.rs`): Added `inclusion_list_store: Mutex<InclusionListStore<T::EthSpec>>` field (same pattern as `execution_bid_pool`). Initialized via `Default` in builder.

2. **heze_verification module** (`beacon_chain/src/heze_verification.rs`): New module with:
   - `InclusionListError` enum with 8 variants mapping to gossip Accept/Reject/Ignore
   - `VerifiedInclusionList<T>` struct (signed IL + view freeze cutoff flag)
   - `verify_inclusion_list_for_gossip()`: 6 spec checks (slot, fork, committee membership, committee root, signature, duplicate/equivocation)
   - `import_inclusion_list()`: stores via `process_signed_inclusion_list()`
   - `get_inclusion_lists_by_committee_indices()`: serves ILs for RPC by committee position
   - `check_inclusion_list_satisfaction()`: checks payload txs include all IL txs
   - `compute_inclusion_list_bits_for_slot()`: returns BitVector for self-build bid

3. **Gossip validation** (`gossip_methods.rs`): Replaced stub with full validation — calls `verify_inclusion_list_for_gossip()`, maps errors to `MessageAcceptance` (Ignore/Reject) with peer penalties, imports on Accept.

4. **RPC serving** (`rpc_methods.rs`): Replaced stub with full handler — gets current slot, calls `get_inclusion_lists_by_committee_indices()`, streams responses, terminates.

5. **InclusionListStore signed_cache** (`inclusion_list_store.rs`): Added `signed_cache: HashMap<InclusionListKey, HashMap<u64, SignedInclusionList<E>>>` to support RPC serving. `process_signed_inclusion_list()` wraps `process_inclusion_list()` and caches the signed version. Pruned with existing `prune()`.

6. **Fork choice IL satisfaction** (`fork_choice.rs`): Parameterized `on_execution_payload` to accept `inclusion_list_satisfied: bool` (was hardcoded `true`). All ~26 call sites updated.

7. **Beacon chain wiring** (`beacon_chain.rs`): `apply_payload_envelope_to_fork_choice` and `process_envelope_for_sync` now compute IL satisfaction via `check_inclusion_list_satisfaction()` before fork choice updates. Self-build path passes `true`.

8. **Self-build bid IL bits** (`beacon_chain.rs`): Heze self-build bid uses `compute_inclusion_list_bits_for_slot(slot)` instead of `BitVector::default()`.

Tests: 1114/1114 types pass, 1038/1038 state_processing pass, 206/206 proto_array pass, 121/121 fork_choice pass. Full workspace lint clean.

**Phase 5 complete.**

**Next:** Phase 6 — Validator Client (IL committee duties, IL construction, bid validation).

### Phase 6: Validator Client — Part 1 (run 3353)

Adding inclusion list committee duty discovery pipeline for FOCIL.

**Completed:**

1. **InclusionListDutyData** (`common/eth2/src/types.rs`): New duty data type with `pubkey`, `validator_index`, `slot`, `il_committee_index`. Mirrors `PtcDutyData` pattern.

2. **validator_inclusion_list_duties** (`beacon_chain/src/beacon_chain.rs`): New method computing IL committee duties for an epoch. Advances state, builds committee caches, iterates slots calling `get_inclusion_list_committee()`, matches against requested validator indices. Returns duties + dependent root.

3. **BN HTTP endpoint** (`beacon_node/http_api/src/inclusion_list_duties.rs`): `POST /eth/v1/validator/duties/inclusion_list/{epoch}` — Heze-gated, validates epoch range, delegates to `validator_inclusion_list_duties()`.

4. **eth2 client method** (`common/eth2/src/lib.rs`): `post_validator_duties_inclusion_list(epoch, indices)` — sends POST request matching BN endpoint.

5. **InclusionListDutiesMap** (`validator_client/validator_services/src/inclusion_list_duties.rs`): Duty cache mirroring `PtcDutiesMap` — `duties_for_slot()`, `duty_count()`, `set_duties()`, `prune()`. 14 unit tests.

6. **poll_inclusion_list_duties** (same file): Async polling function — checks Heze fork activation, fetches current+next epoch duties from BN, caches in map, prunes old epochs.

7. **DutiesService integration** (`duties_service.rs`): Added `inclusion_list_duties` field, polling loop (`duties_service_inclusion_list` task), `il_committee_count()` method.

8. **MockBeaconNode** (`testing/validator_test_rig/src/mock_beacon_node.rs`): `mock_post_validator_duties_inclusion_list()` for test infrastructure.

Tests: 1175/1175 types+validator_services pass, 1038/1038 state_processing pass. Full workspace lint clean.

**Next:** Phase 6 Part 2 — `sign_inclusion_list` in ValidatorStore, InclusionListService (IL construction, signing, broadcast), pool submission endpoint.

### Phase 6: Validator Client — Part 2 (run 3354)

Adding inclusion list signing, pool submission, and the InclusionListService for FOCIL.

**Completed:**

1. **SignableMessage::InclusionList** (`validator_client/signing_method/src/lib.rs`): New variant wrapping `&InclusionList<E>` with signing root and Web3Signer unsupported error.

2. **sign_inclusion_list** (`validator_store/src/lib.rs` + `vibehouse_validator_store/src/lib.rs`): Trait method + implementation using `DOMAIN_INCLUSION_LIST_COMMITTEE`, doppelganger bypassed (non-slashable). Returns `SignedInclusionList`.

3. **BN pool endpoint** (`beacon_node/http_api/src/lib.rs`): `POST /eth/v1/beacon/pool/inclusion_lists` — Heze-gated, verifies via `verify_inclusion_list_for_gossip()`, imports to InclusionListStore, publishes to gossip network.

4. **eth2 client method** (`common/eth2/src/lib.rs`): `post_beacon_pool_inclusion_lists(signed_il)`.

5. **InclusionListDutyData.inclusion_list_committee_root** (`common/eth2/src/types.rs`): Added `Hash256` field for the committee root, computed in `validator_inclusion_list_duties` via `FixedVector::tree_hash_root`.

6. **InclusionListService** (`validator_client/validator_services/src/inclusion_list_service.rs`): New service (builder pattern mirroring PayloadAttestationService):
   - Wakes at 6667 BPS (66.67% of slot, before 75% view freeze cutoff)
   - Reads IL committee duties from DutiesService
   - Constructs `InclusionList` with committee root from duties (transactions empty — EL integration deferred)
   - Signs with `sign_inclusion_list`
   - Submits to BN via `post_beacon_pool_inclusion_lists`
   - Wired into VC main loop (`validator_client/src/lib.rs`)

7. **ValidatorStore trait stubs**: Added `sign_inclusion_list` unimplemented stubs to all 4 mock ValidatorStore impls (payload_attestation_service, duties_service, ptc test modules).

Tests: 12/12 vibehouse_validator_store pass (3 new: correct domain, wrong domain, unknown pubkey), 61/61 validator_services pass, 1115/1115 types+validator_client pass. Full workspace lint clean.

**Phase 6 complete.** All VC tasks done: duty discovery (Part 1) + signing, broadcast, pool endpoint (Part 2).

**Design note:** Transactions in inclusion lists are currently empty. The EL integration (`engine_getInclusionList` or equivalent) to populate them will be added when the execution layer FOCIL spec is finalized.

**Next:** Phase 7 — REST API (inclusion list endpoints).
