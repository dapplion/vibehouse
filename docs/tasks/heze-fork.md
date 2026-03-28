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
| 7. REST API | IL endpoints | DONE |

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

### Phase 7: REST API (run 3355)

Completing the REST API for Heze FOCIL inclusion lists.

**Already implemented in Phase 6:**
- `POST /eth/v1/beacon/pool/inclusion_lists` — submit signed IL for gossip (Phase 6 Part 2)
- `POST /eth/v1/validator/duties/inclusion_list/{epoch}` — IL committee duty discovery (Phase 6 Part 1)

**Completed:**

1. **GET /eth/v1/beacon/pool/inclusion_lists** (`beacon_node/http_api/src/lib.rs`): Returns all signed inclusion lists from the InclusionListStore. Optional `?slot=N` query parameter for filtering. Mirrors `get_beacon_pool_payload_attestations` pattern.

2. **BeaconChain::get_all_inclusion_lists** (`beacon_chain.rs`): Public method iterating `inclusion_list_store.signed_cache`, with optional slot filter. Returns `Vec<SignedInclusionList>`.

3. **InclusionListPoolQuery** (`common/eth2/src/types.rs`): Query struct with `slot: Option<Slot>`.

4. **eth2 client method** (`common/eth2/src/lib.rs`): `get_beacon_pool_inclusion_lists(slot: Option<Slot>)` returning `GenericResponse<Vec<SignedInclusionList<E>>>`.

Tests: all pool tests pass (fulu), 26/26 inclusion list types tests pass. Clean clippy.

**Phase 7 complete.** All Heze fork phases are now DONE.

### Test coverage improvement (run 3356)

Added 3 tests for Heze inclusion list satisfaction blocking in `should_extend_payload`:
- `should_extend_payload_inclusion_list_not_satisfied_blocks_extension`: envelope received + PTC quorum met BUT IL not satisfied → must block (critical EIP-7805 path)
- `should_extend_payload_inclusion_list_not_satisfied_no_envelope_allows_extension`: no envelope → IL check skipped
- `should_extend_payload_inclusion_list_satisfied_allows_ptc_path`: IL satisfied → PTC path proceeds normally

This was the only untested branch in should_extend_payload's Heze logic. 209/209 proto_array + 121/121 fork_choice tests pass.

### Test coverage improvement (run 3500+)

Added 7 tests for `process_signed_inclusion_list()` signed cache behavior in `inclusion_list_store.rs`:
- `signed_process_caches_accepted_il`: accepted IL appears in both inclusion_lists and signed_cache
- `signed_process_not_cached_after_cutoff`: after view freeze cutoff, not stored or cached
- `signed_process_equivocation_removes_from_cache`: equivocation cleans up signed_cache
- `signed_process_duplicate_idempotent`: duplicate submission stays at one entry
- `signed_process_multiple_validators`: multiple validators each get their own cache entry
- `prune_removes_signed_cache`: prune cleans signed_cache along with inclusion_lists and equivocators
- `signed_equivocator_ignored_on_third_attempt`: third submission from equivocator is ignored

Total: 20/20 inclusion_list_store tests pass (13 existing + 7 new).

### CI: update FORK_NAME to heze (run 3501+)

Updated CI to test Heze as the latest fork (was Gloas):
- `ci.yml`: beacon_chain, http_api, network, op_pool tests now use `FORK_NAME=heze`
- `nightly-tests.yml`: added `gloas` to nightly fork matrix (since it's no longer tested on push)
- `Makefile`: added `heze` to `FORKS` and `RECENT_FORKS` lists

Fixed 4 test failures in `network_beacon_processor/tests.rs` for Heze compatibility:
1. `test_blobs_by_range_spans_fulu_fork`: added `heze_fork_epoch = Some(Epoch::new(3))`
2. `test_gloas_gossip_bid_duplicate_ignored`: changed `as_gloas_mut()` → `builders_mut()` for state access
3. `test_gloas_gossip_proposer_preferences_fork_boundary_*`: added `heze_fork_epoch = Some(Epoch::new(2))` in `pre_gloas_rig`
4. `test_gloas_gossip_payload_envelope_invalid_signature_rejected`: changed `as_gloas()` → `latest_block_hash()` for state access

Created fork-aware helpers `bid_for_fork()` and `unsigned_bid()` to construct correct `ExecutionPayloadBid` variant (Gloas or Heze) based on FORK_NAME. Refactored `sign_bid()` to accept the superstruct enum. Changed assertion patterns from `.as_gloas().unwrap().message.field` to `.to_ref().message().field()` for fork-generic field access.

Test results with FORK_NAME=heze: beacon_chain 999/999, http_api 345/345, network 205/205, op_pool 72/72. EF tests 142/142 + 80/80.

### Devnet testing preparation (run 3500+)

Added Heze devnet support to kurtosis infrastructure:

1. **`kurtosis/vibehouse-heze.yaml`**: New config with `gloas_fork_epoch: 1` and `heze_fork_epoch: 3`. Tests both fork transitions on 4 nodes.

2. **`scripts/kurtosis-run.sh --heze`**: New mode using ethereum-package `main` branch (commit 173e3d5c32ca which includes heze support). The v6.0.0 release only supports gloas; main has full heze genesis generation.

3. **Health check**: Updated fork tracking to distinguish `gloas` vs `heze` based on `HEZE_FORK_SLOT`.

4. **`ETHEREUM_PACKAGE`** variable: Script now uses a configurable package reference instead of hardcoded v6.0.0. Default remains v6.0.0 for all non-heze modes.

**Note:** Heze is a CL-only change (FOCIL). The EL (geth epbs-devnet-0) doesn't need changes — inclusion list transactions are empty until the EL FOCIL spec is finalized. The devnet tests the fork transition, state upgrade, committee computation, and IL gossip/RPC plumbing.

### Proposer-side bid IL bits validation (run 3502+)

Added `is_inclusion_list_bits_inclusive` check in `get_best_execution_bid` per Heze validator.md spec requirement. When selecting a builder bid for block production:

- For Heze bids, computes inclusion list committee for `slot - 1`
- Checks `is_inclusion_list_bits_inclusive(store, committee, committee_root, slot-1, bid.inclusion_list_bits)`
- Bids failing the check are rejected (falls back to self-build)
- Only activates for Heze bids; Gloas bids unaffected

This ensures the proposer only accepts bids from builders that have observed at least all the inclusion lists the proposer has locally seen. Tests: 999/999 beacon_chain (heze), 999/999 beacon_chain (gloas).

### Heze devnet verification (run 3503+)

Successfully ran 4-node heze devnet (`scripts/kurtosis-run.sh --heze`). Results:

- **Fork transitions**: genesis → Gloas (epoch 1, slot 8) → Heze (epoch 3, slot 24) — both clean
- **Finalization**: reached finalized_epoch=8 (slot 80, epoch 10) in 468s
- **Chain health**: continuous finalization from epoch 2 through epoch 8, no stalls
- **All 4 nodes**: healthy, synced, not stalling

Fixed two infrastructure issues:
1. Kurtosis couldn't resolve pinned commit hash `173e3d5c32ca` — switched to `@main` branch reference
2. Updated dora image from `gloas-support` to `heze-support` for correct fork display

### Gossip validation spec compliance fix (run 3600+)

Fixed 3 spec compliance gaps in `inclusion_list` gossip validation (p2p-interface.md):

1. **Accept previous-slot ILs** (spec conditions 2+3): Was only accepting `il_slot == current_slot`. Now accepts previous slot with timing check: previous slot ILs are accepted only before `get_attestation_due_ms(epoch)` into the current slot.

2. **`MAX_BYTES_PER_INCLUSION_LIST` check** (spec condition 1): Added `[REJECT]` for inclusion lists where total transaction bytes exceed 8192.

3. **Committee root mismatch → IGNORE** (spec condition 4): Was incorrectly `[REJECT]` with peer penalty. Changed to `[IGNORE]` since committee root depends on the peer's chain view (different head → different committee), not malice.

Tests: 205/205 network (heze), 20/20 inclusion_list_store, 17/17 EF operations+SSZ, 5/5 heze state_processing. Zero clippy warnings.

### Network-level gossip validation tests (run 3601+)

Added 6 network-level integration tests for the inclusion list gossip validation pipeline in `network_beacon_processor/tests.rs`:

1. `test_heze_gossip_inclusion_list_valid_accepted`: valid IL from committee member → Accept
2. `test_heze_gossip_inclusion_list_wrong_slot_rejected`: far-future slot → Reject
3. `test_heze_gossip_inclusion_list_not_in_committee_rejected`: non-committee validator → Reject
4. `test_heze_gossip_inclusion_list_wrong_committee_root_ignored`: mismatched committee root → Ignore
5. `test_heze_gossip_inclusion_list_invalid_signature_rejected`: bad signature → Reject
6. `test_heze_gossip_inclusion_list_duplicate_ignored`: second identical submission → Ignore

Created `heze_rig()`, `make_inclusion_list()`, and `sign_inclusion_list()` test helpers following the existing Gloas gossip test patterns. Added `tree_hash` dev-dependency to network crate for committee root computation.

Tests: 211/211 network (heze), 60/60 Gloas gossip tests (gloas). Zero clippy warnings.

### Fix IL satisfaction slot offset (run 3823+)

Fixed spec compliance bug in `check_inclusion_list_satisfaction` and `compute_inclusion_list_bits_for_slot` — both were using `envelope.slot` / `slot` directly instead of `slot - 1`.

Per the Heze fork-choice spec, `record_payload_inclusion_list_satisfaction` uses `Slot(state.slot - 1)` — inclusion lists broadcast at slot N-1 constrain the payload at slot N. Similarly, the validator spec requires `is_inclusion_list_bits_inclusive(store, state, slot - 1, bits)` for bid inclusion_list_bits.

The existing `get_best_execution_bid` already correctly used `slot - 1` for the `is_inclusion_list_bits_inclusive` check, but the satisfaction check and self-build bits computation were off by one slot.

Fix: both functions now compute `il_slot = slot - 1` and use it for committee computation and IL store lookups. Slot 0 gracefully returns true/default (no previous slot).

Tests: 86/86 EF (real crypto) + 148/148 EF (fake crypto) + 209 proto_array + 33 inclusion_list_store — all pass.
