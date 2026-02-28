# Code Review & Quality Improvement

## Goal
The loop has shipped a massive amount of code autonomously (100+ runs). Review the codebase for quality issues, technical debt, and correctness concerns that may have accumulated during rapid autonomous development.

## Strategy

### Phase 1: Audit & Inventory
- [x] Run `cargo clippy --workspace -- -W clippy::all` and fix all warnings
- [x] Run `cargo doc --workspace --no-deps` — fix any doc warnings
- [x] Identify dead code, unused imports, unreachable paths
- [x] Check for `unwrap()`/`expect()` in non-test code — replace with proper error handling
- [x] Look for `todo!()`, `unimplemented!()`, `fixme`, `hack` comments

### Phase 2: Architecture Review — DONE
- [x] Review public API surface — are things `pub` that shouldn't be?
- [x] Check module organization — any god-files that should be split?
- [x] Review error types — consistent error hierarchy? Good error messages?
- [x] Check for code duplication across Gloas fork variants
- [x] Review superstruct variant handling — any missing arms, fallthrough bugs?

### Phase 3: Correctness Deep-Dive — DONE
- [x] Cross-reference Gloas implementation against consensus-specs v1.7.0-alpha.2
- [x] Verify all spec constants match (domain types, config values, timing)
- [x] Review edge cases in state transitions — overflow, underflow, empty collections
- [x] Audit builder payment/withdrawal logic for economic bugs
- [x] Review fork choice weight calculations against spec

### Phase 4: Performance — DONE
- [x] Profile hot paths (state transition, block processing, attestation validation)
- [x] Check for unnecessary clones, allocations in tight loops
- [x] Review database access patterns — any N+1 queries?
- [x] Check serialization/deserialization efficiency

### Phase 5: Test Quality — DONE
- [x] Review test coverage gaps — which critical paths lack tests?
- [x] Check test assertions — are they testing the right things?
- [x] Look for flaky/non-deterministic tests
- [x] Ensure integration tests cover realistic scenarios

## Process
Each loop iteration should:
1. Pick one sub-task from the checklist above
2. Audit the relevant code
3. Fix issues found (with tests)
4. Commit fixes atomically
5. Document findings and decisions here

## Findings Log

### Run 217: unwrap/expect audit + todo/fixme/hack scan

**Scope**: Full codebase scan for `unwrap()`, `expect()`, `todo!()`, `unimplemented!()`, `FIXME`, `HACK` in production code.

**Results — todo/unimplemented**:
- Zero `todo!()` macros in production code
- Zero `unimplemented!()` in production code (all in `#[cfg(test)]` mock trait impls)

**Results — FIXME/HACK** (all pre-existing, inherited from Lighthouse):
- `task_executor/src/lib.rs:87` — dead `service_name` field. Removing would be noisy (touches many callers), low value.
- `slasher/src/database/lmdb_impl.rs:168` — LMDB bindings bug workaround. Intentional.
- `lighthouse_validator_store/src/lib.rs:188` — `clippy::await_holding_lock` suppression pending async lock refactor. Known tech debt.
- `types/src/chain_spec.rs:1726` — `skip_serializing` hack for blob schedule. Can remove after Fulu is live.
- `operation_pool/src/lib.rs:286` — Electra cross-committee aggregation. Needs cleaner design but works correctly.
- `lighthouse/src/main.rs:84` — build profile name extraction from OUT_DIR. Intentional pattern.
- `network_utils/src/unused_port.rs:44` — port allocation with inherent TOCTOU. Known, acceptable for testing utility.

**Results — unwrap/expect in production code**:
- All Gloas consensus code (state_processing, envelope_processing, fork_choice, block_verification) is clean — uses `?` and `map_err` throughout.
- `beacon_chain.rs:7048` — `Signature::infinity().expect(...)` in self-build block production. **Fixed**: replaced with `map_err` + `?` propagation via `BlockProductionError::InvalidBlockVariant`.
- `proto_array_fork_choice.rs:1125` — `.unwrap()` on `max_by()` in head selection. **Safe**: guarded by `children.is_empty()` check 3 lines above. Comment documents invariant.
- `custody_context.rs:319-453` — `.expect()` on `sampling_size_custody_groups()`. Currently infallible but fragile. Pre-existing, not Gloas-specific.
- `subnet_service/mod.rs:645,664` — `.expect("Waker has been set")` in `Stream::poll_next`. Safe by control flow. Pre-existing.
- `naive_aggregation_pool.rs:52-56` — `.expect()` in `TreeHash` impl. Safe (exact leaf count). Pre-existing.
- `chain_config.rs:27` — `.expect()` on static hex constant. Effectively infallible. Pre-existing.
- `beacon_chain.rs:8511-8616` — `dump_as_dot`/`dump_dot_file` debug utilities. Dead code, acceptable for diagnostics.

**Decision**: Fixed the one production unwrap in our Gloas code. All other findings are pre-existing Lighthouse patterns that are either safe by invariant or intentional. No action needed on those.

### Run 218: clippy audit + cargo doc warnings fix

**Scope**: Full `cargo clippy --workspace --all-targets` audit and `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.

**Clippy results**: Zero warnings. Codebase is fully clippy-clean.

**Cargo doc results** — fixed all warnings across 79 files:
- **127 bare URLs** in doc comments wrapped in angle brackets (`<https://...>`) across 74 files
- **3 `[Gloas]` references** escaped as `\[Gloas\]` to prevent broken intra-doc links (fork_choice.rs)
- **21 `\`\`\`ignore` code blocks** changed to `\`\`\`text` for non-Rust content (ASCII diagrams, shell commands, directory trees) across 13 files
- **1 unclosed HTML tag** — `FixedBytes<N>` wrapped in backticks (fixed_bytes/src/lib.rs)
- **1 broken intra-doc link** — `[ChainSpec::compute_fork_digest]` changed to backtick-quoted (enr_fork_id.rs)
- **1 broken self-reference** — `[self::sampling_columns_for_epoch]` simplified (custody_context.rs)
- **1 broken link to private item** — `[ProtocolQuota]` changed to backtick-quoted (rpc/config.rs)
- **1 broken link to `Rpc`** — backtick-quoted (rpc/mod.rs)
- **2 broken bracket patterns** — `[5,13,21]` in hdiff.rs wrapped in backticks
- **2 bracket patterns** — `[tcp,udp,quic]` and `[tcp6,udp6,quic6]` escaped (enr_ext.rs)

**Result**: `cargo doc --workspace --no-deps` passes with `-D warnings`. `cargo clippy` clean. 2417/2425 tests pass (8 web3signer timeouts are pre-existing infrastructure-dependent failures).

### Run 219: dead code audit + spec conformance review

**Scope**: Phase 1 dead code audit + Phase 3 partial correctness deep-dive.

**Dead code results**:
- `#[allow(dead_code)]` annotations: ~60 instances found, ALL in pre-existing Lighthouse code or test infrastructure. Zero in Gloas-specific code.
- `#[allow(unused_imports)]`: 3 instances, all in macro-generated code in `signed_beacon_block.rs`. Pre-existing.
- All Gloas public functions (9 in `gloas.rs`, 1 in `envelope_processing.rs`, 1 in `per_epoch_processing/gloas.rs`, 13+ in `beacon_chain.rs`) verified as actively called in production code paths.
- No dead code found. Phase 1 complete.

**Spec conformance review — cross-referenced against consensus-specs/gloas**:

1. **`process_execution_payload_bid`** ✓ — matches spec: self-build validation (amount=0, G2_POINT_AT_INFINITY), builder active check, `can_builder_cover_bid` (MIN_DEPOSIT_AMOUNT + pending), signature verification, blob commitment limit, slot/parent_hash/parent_root/prev_randao checks, pending payment recording at `SLOTS_PER_EPOCH + slot % SLOTS_PER_EPOCH`, bid caching.

2. **`process_payload_attestation`** ✓ — matches spec: beacon_block_root == parent_root, slot+1 == state.slot, get_indexed_payload_attestation → is_valid_indexed_payload_attestation (sorted indices, non-empty, aggregate signature).

3. **`process_execution_payload_envelope`** ✓ — matches spec order exactly: (1) signature verification, (2) cache state root in block header, (3) verify beacon_block_root/slot, (4) verify committed_bid consistency (builder_index, prev_randao), (5) verify withdrawals hash, (6) verify gas_limit/block_hash/parent_hash/timestamp, (7) process execution requests, (8) queue builder payment, (9) set execution_payload_availability + update latest_block_hash, (10) verify state root.

4. **`process_builder_pending_payments`** ✓ — matches spec: quorum = per_slot_balance * numerator / denominator, check first SLOTS_PER_EPOCH payments against quorum, rotate second half to first half, clear second half.

5. **`get_ptc_committee`** ✓ — matches spec: seed = hash(get_seed + slot_bytes), concatenate all committees, compute_balance_weighted_selection with shuffle_indices=False.

6. **Fork choice `validate_on_attestation`** ✓ — matches spec Gloas additions: index must be 0 or 1 for Gloas blocks, same-slot must be index 0, index=1 requires payload_revealed.

7. **Fork choice `get_gloas_weight`** ✓ — matches spec: non-PENDING nodes at adjacent slot (slot+1==current) return 0, otherwise sum attestation scores + optional proposer boost.

8. **Fork choice `find_head_gloas`** ✓ — matches spec get_head: start at justified, loop picking max(weight, root, tiebreaker) from children.

9. **`process_withdrawals_gloas`** ✓ — matches spec order: (1) builder pending withdrawals capped at MAX-1, (2) partial validator withdrawals capped at MAX-1, (3) builder sweep capped at MAX-1, (4) validator sweep capped at MAX. All state updates (apply_withdrawals, update indices, store expected_withdrawals) verified correct.

**No spec divergences found.** All checked functions match the consensus-specs faithfully.

### Run 220: spec constants verification

**Scope**: Phase 3 sub-task — verify all Gloas spec constants match consensus-specs v1.7.0-alpha.2 (domain types, preset values, config values, timing parameters, fork versions).

**Method**: Fetched spec from `ethereum/consensus-specs/master` (beacon-chain.md, fork-choice.md, validator.md, presets/mainnet/gloas.yaml, presets/minimal/gloas.yaml, configs/mainnet.yaml) and cross-referenced against vibehouse codebase.

**Results — all constants verified correct**:

| Category | Constants Checked | Status |
|----------|------------------|--------|
| Domain types | DOMAIN_BEACON_BUILDER (0x0B), DOMAIN_PTC_ATTESTER (0x0C), DOMAIN_PROPOSER_PREFERENCES (0x0D) | ✓ |
| Index flags | BUILDER_INDEX_FLAG (2^40), BUILDER_INDEX_SELF_BUILD (UINT64_MAX) | ✓ |
| Builder params | payment threshold 6/10, withdrawal prefix 0x03, min withdrawability delay 64 epochs | ✓ |
| Preset (mainnet) | PTC_SIZE=512, MAX_PAYLOAD_ATTESTATIONS=4, BUILDER_REGISTRY_LIMIT=2^40, BUILDER_PENDING_WITHDRAWALS_LIMIT=2^20, MAX_BUILDERS_PER_WITHDRAWALS_SWEEP=2^14 | ✓ |
| Preset (minimal) | PTC_SIZE=2, MAX_PAYLOAD_ATTESTATIONS=4, MAX_BUILDERS_PER_WITHDRAWALS_SWEEP=16 | ✓ |
| Fork choice | PAYLOAD_TIMELY_THRESHOLD=PTC_SIZE//2 (>), DATA_AVAILABILITY_TIMELY_THRESHOLD=PTC_SIZE//2 (>), PayloadStatus enum (0/1/2) | ✓ |
| Timing BPS | attestation=2500, aggregate=5000, sync=2500, contribution=5000, payload_attestation=7500 | ✓ |
| Fork versions | mainnet [0x07,0,0,0], minimal [0x07,0,0,1], gnosis [0x07,0,0,0x64] | ✓ |
| Networking | MAX_REQUEST_PAYLOADS=128 | ✓ |
| Derived types | BuilderPendingPaymentsLimit (2*SLOTS_PER_EPOCH per spec), ProposerLookaheadSlots | ✓ |

**Bug found and fixed**:
- `GnosisEthSpec::MaxPayloadAttestations` was `U2` but the gnosis preset yaml and ChainSpec both say 4. This would have limited Gnosis Gloas blocks to 2 payload attestations instead of 4. **Fixed**: changed to `U4` in `eth_spec.rs:662`. All 711 types tests + 69 SSZ static EF tests pass.

### Run 221: architecture review — superstruct variants, code duplication, error types

**Scope**: Phase 2 sub-tasks: superstruct variant handling, code duplication across Gloas fork variants, error type consistency.

**Superstruct variant handling audit**:
- All primary superstruct types include Gloas variants (BeaconBlock, BeaconBlockBody, BeaconState, ExecutionPayload, ExecutionPayloadHeader, BuilderBid, SignedBeaconBlock, LightClientUpdate, LightClientHeader, LightClientBootstrap, DataColumnSidecar)
- All `ForkName` match expressions explicitly handle Gloas — no missing arms
- Intentional field omissions documented: `blob_kzg_commitments` removed from Gloas body (moved to bid), `execution_requests` removed (moved to envelope)
- Wildcard `_ =>` patterns audited — none silently catching Gloas in consensus-critical paths
- **No issues found.**

**Code duplication audit**:
- Superstruct deserialization arms (Fulu vs Gloas): identical logic but framework requires separate arms. Cannot consolidate — superstruct limitation.
- Test helpers (`make_gloas_state`/`make_fulu_state`): intentionally different (ePBS-specific fields).
- RPC protocol limits already correctly grouped: `Electra | Fulu | Gloas`.
- **No actionable duplication found.**

**Error type consistency audit**:
- `BeaconChainError::EnvelopeProcessingError(String)` was wrapping `state_processing::EnvelopeProcessingError` via `format!("{:?}", e)`, losing structured error information.
- **Fixed**: Split into two variants:
  - `EnvelopeProcessingError(EnvelopeProcessingError)` — wraps the structured state_processing error type directly
  - `EnvelopeError(String)` — for ad-hoc beacon_chain-level envelope errors (missing blocks, newPayload failures, etc.)
- The two `process_execution_payload_envelope` call sites now use `.map_err(Error::EnvelopeProcessingError)?` instead of `format!("{:?}", e)`
- Ad-hoc string errors (13 call sites) migrated to `EnvelopeError`
- `BlockProductionError::EnvelopeConstructionFailed(String)` kept as-is — mixed usage prevents clean migration
- All 285 Gloas beacon_chain tests pass, clippy clean, fmt clean

**Phase 2 checklist update**:
- [x] Review superstruct variant handling — any missing arms, fallthrough bugs?
- [x] Check for code duplication across Gloas fork variants
- [x] Review error types — consistent error hierarchy? Good error messages?
- [x] Review public API surface — are things `pub` that shouldn't be?
- [x] Check module organization — any god-files that should be split?

### Run 222: module organization audit, public API surface, state transition edge cases

**Scope**: Phase 2 (module organization, public API surface) + Phase 3 (state transition edge cases). Completes both phases.

**Module organization audit — god-file analysis**:

Top files by line count:
| File | Lines | Notes |
|------|-------|-------|
| `tests/gloas.rs` | 12,588 | Test file — large but splitting tests has limited benefit |
| `beacon_chain.rs` | 8,805 | Classic god-file, pre-existing. Hard to split (tightly coupled `self` methods) |
| `proto_array_fork_choice.rs` | 6,934 | Fork choice with ePBS additions. Pre-existing structure |
| `per_block_processing/gloas.rs` | 5,936 | ~1010 prod + ~4926 tests. Production code is well-organized into bid/attestation/withdrawal/utility groups |

**Decision**: No splits needed. The largest Gloas file (`per_block_processing/gloas.rs`) has only ~1010 lines of production code — the bulk is tests. The production functions group naturally (bid processing, payload attestations, withdrawals, builder utils). Splitting would create unnecessary indirection without improving readability. The pre-existing god-files (`beacon_chain.rs`, `proto_array_fork_choice.rs`) are inherited and not Gloas-specific.

**Public API surface audit**:

Audited all `pub` items in 5 Gloas files. Most are correctly `pub` — used by external crates (ef_tests, beacon_chain, http_api, network).

**Fixed — 2 functions downgraded to `pub(crate)`**:
- `get_pending_balance_to_withdraw_for_builder` in `per_block_processing/gloas.rs` — only used within `state_processing` crate (by `verify_exit.rs` and internal tests)
- `upgrade_state_to_gloas` in `upgrade/gloas.rs` — only called by `upgrade_to_gloas` in the same file

All other `pub` items verified as legitimately needed by external crates.

**State transition edge cases audit**:

Comprehensive audit of all Gloas code in `consensus/state_processing/src/` for:

| Category | Status | Details |
|----------|--------|---------|
| Arithmetic overflow/underflow | SAFE | All `+`, `-`, `*`, `/` use `safe_arith` (`safe_add`, `saturating_add`, `safe_div`, `safe_rem`, `safe_mul`) |
| Division by zero | SAFE | All divisors explicitly checked before use (`builders_count > 0`, `validators_len > 0`, `indices.is_empty()` guards) |
| Array indexing | SAFE | Uses `.get()` consistently instead of `[]` — never direct indexing |
| Empty collections | SAFE | Proper `.is_empty()` and `.last().map().unwrap_or()` patterns |
| Builder/validator index bounds | SAFE | Proactive validation with `.get()` + `.ok_or()` before access |
| Withdrawal index wrapping | SAFE | Uses `safe_rem()` for circular sweeps |
| Envelope payload/state consistency | SAFE | Verifies alignment before processing |

**No issues found.** The Gloas state transition code demonstrates consistently defensive programming — safe arithmetic, bounds checking, zero-divisor guards, and proper error propagation throughout.

**Phase 2 and Phase 3 are now complete.**

### Run 223: performance audit — hot paths, clones, allocations

**Scope**: Phase 4 sub-tasks: profile hot paths for unnecessary clones/allocations, review database access patterns, check serialization efficiency.

**Method**: Three parallel agent searches across state_processing (block/envelope/epoch), proto_array fork choice, and beacon_chain integration. Identified all `.clone()` calls in Gloas-specific code, categorized as necessary vs unnecessary.

**Fixed — 2 performance improvements in `process_withdrawals_gloas`**:

1. **`withdrawals.clone()` eliminated** (line 707): The entire `withdrawals` Vec was cloned to create `payload_expected_withdrawals` List, then used only for `.len()` and `.last()` comparison afterward. **Fix**: capture `withdrawals_len` and `last_validator_index` before consuming `withdrawals` by value into `List::new()`. Saves one full Vec clone per block.

2. **`builder_pending_withdrawals` reconstruction replaced with `pop_front`** (lines 715-722): Was cloning all remaining items via `.iter().skip(n).cloned().collect()` into a new Vec, then `List::new()`. **Fix**: use milhouse `List::pop_front()` for in-place removal (same method already used for `pending_partial_withdrawals` on line 729). Avoids heap allocation + element cloning.

**Audited but not changed (necessary clones or pre-existing patterns)**:

| Category | Finding | Action |
|----------|---------|--------|
| `payment.withdrawal.clone()` (epoch processing) | Required — can't borrow `builder_pending_payments` and mutably push to `builder_pending_withdrawals` simultaneously | None (borrow checker constraint) |
| `new_balances.clone()` (find_head) | Required — `new_balances` is `&JustifiedBalances`, must clone to store | None (API constraint) |
| `bid.clone()` (apply_execution_bid) | Required — bid pool takes ownership, caller needs the value too | None |
| `get_best_bid().cloned()` | Required — returns owned value from locked pool | None |
| Proto_array child finding O(n) scan | Pre-existing algorithm, tree is pruned at finality (~few hundred nodes) | Future optimization opportunity |
| `Vec<&PublicKey>` in signature verification | Required by BLS API (`fast_aggregate_verify` takes `&[&PublicKey]`); blst also collects internally. PTC_SIZE=512 → 4KB | None |
| `compute_filtered_roots` HashSet | Required for O(1) lookup in `get_gloas_children` | None |
| Epoch processing rotation clones | Element-level clones for same-list src/dst copy, unavoidable with milhouse API | None |
| Beacon_chain envelope state clone | Required — must mutate state copy for envelope processing without affecting original | None |

**Database access patterns**: No N+1 queries found. State access in Gloas code goes through milhouse `List::get()` which is O(1) tree access. Validator lookups use `state.validators().get(i)` which is direct indexed. No unbounded queries.

**Serialization efficiency**: Gloas types use SSZ (via `ssz_derive`) throughout. No custom serialization. `tree_hash_root()` is called only where needed (signing roots, state roots). No unnecessary re-serialization.

**Test results**: 272/272 Gloas state_processing tests pass, 309/309 beacon_chain Gloas integration tests pass, EF spec withdrawal + sanity tests pass. Clippy clean.

### Run 224: test quality review — coverage, assertions, flakiness

**Scope**: Phase 5 — audit test coverage gaps, assertion quality, non-deterministic patterns, and integration test realism across all Gloas code.

**Method**: Three parallel agent searches across state_processing (175+ tests), beacon_chain integration (309+ tests), fork choice (51+ tests), HTTP API (39+ tests), and network processor (41+ Gloas tests).

**Coverage assessment — no gaps in Gloas-specific code**:

| Module | Tests | Coverage |
|--------|-------|----------|
| `per_block_processing/gloas.rs` | ~91 | All 9 public functions tested with edge cases |
| `envelope_processing.rs` | ~28 | All validation paths + state mutations tested |
| `per_epoch_processing/gloas.rs` | ~21 | Quorum threshold boundaries, rotation mechanics |
| `upgrade/gloas.rs` | ~26 | Complete Fulu→Gloas migration coverage |
| `per_slot_processing.rs` (Gloas) | ~8 | Availability bit clearing + integration |
| `beacon_chain/tests/gloas.rs` | ~231 | Chain finalization, block production, envelope lifecycle |
| `gloas_verification.rs` tests | ~52 | Gossip validation for bids, attestations, envelopes |
| `proto_array` (Gloas) | ~51 | Head selection, weight, tiebreaker, payload status |
| `fork_choice` (Gloas) | ~18 | Attestation index validation, head payload status |
| HTTP API (Gloas) | ~39 | All Gloas endpoints: PTC duties, envelopes, bids, prefs |
| Network gossip (Gloas) | ~41 | Bid/envelope/attestation/preferences validation |

**Reported "gaps" assessed as non-issues**:

- `PayloadAttestationError::InvalidAggregationBits` — unreachable by construction (`BitVector<PtcSize>` is type-level fixed size, `get(i)` can't OOB when `i < PtcSize`)
- `PayloadAttestationError::PtcCommitteeError` — requires `get_ptc_committee()` internal failure, which would indicate a corrupted beacon state (not a gossip validation concern)
- Event subscription functions (`subscribe_execution_bid/payload_attestation`) — not consensus-critical, SSE event delivery infrastructure
- `get_parent_payload_status_of()`, `get_gloas_children()` — internal helpers thoroughly exercised by 14 head-selection integration tests

**Assertion quality assessment**:

| Module | Quality | Details |
|--------|---------|---------|
| `envelope_processing.rs` tests | EXCELLENT | All tests assert specific state mutations (latest_block_hash, availability bits, balance changes, withdrawal queue contents) |
| `per_block_processing/gloas.rs` tests | GOOD | 239 `assert_eq!` for specific values, 31 `matches!` for error variants, only 1 bare `.is_ok()` |
| `beacon_chain/tests/gloas.rs` | ADEQUATE | Integration tests appropriately rely on chain success/failure; some could assert specific finalized epoch values but this is a style preference, not a bug |

**Flaky test assessment — no Gloas-specific flakiness**:

All timing-sensitive patterns found are in pre-existing inherited code:
- Network processor tests use `STANDARD_TIMEOUT = 10s` and `assert_event_journal_with_timeout()` — inherited from Lighthouse, not Gloas-specific
- `import_gossip_block_acceptably_early()` has a known race condition documented by original author — inherited
- `test_rpc_block_reprocessing()` uses fixed 4s delay + 30ms retry window — inherited

Gloas-specific tests are fully deterministic:
- State processing tests use direct function calls with constructed inputs, no timing
- Fork choice tests use mock slot clocks with explicit slot advancement
- Beacon chain integration tests use test harness with controlled slot progression
- The one Gloas timing test (`gloas_proposer_boost_four_interval_boundary`) uses the mock slot clock's `set_current_time()` — deterministic

**Integration test realism**:

The Gloas integration tests in `beacon_chain/tests/gloas.rs` cover realistic multi-block scenarios:
- Chain finalization through Gloas fork boundary (tests full lifecycle)
- Self-build block production with envelope processing
- External builder path with bid selection and envelope import
- Payload withholding (EMPTY path) and recovery
- Multi-epoch chains with attestation aggregation
- Fork choice head selection with PTC votes and proposer boost

These complement the devnet scenarios (kurtosis scripts) for end-to-end testing.

### Run 225: complete BlockProcessingError envelope error wrapping

**Scope**: Finish the error type improvement started in run 221. Run 221 fixed `BeaconChainError::EnvelopeProcessingError(String)` to wrap the structured type. This run fixes the same pattern in `BlockProcessingError::EnvelopeProcessingError(String)`.

**Changes**:
- `BlockProcessingError::EnvelopeProcessingError(String)` → `EnvelopeProcessingError(Box<EnvelopeProcessingError>)` — preserves structured error information for debugging
- Added `PartialEq` derive to `EnvelopeProcessingError` (required by `BlockProcessingError`'s existing `PartialEq` derive)
- Used `Box` to avoid infinite type recursion (`EnvelopeProcessingError` already contains `BlockProcessingError`)
- Updated 3 call sites: block_replayer (2) + ef_tests operations (1)

**Verification**: 452/452 state_processing tests, 138/138 EF spec tests (fake_crypto), 4/4 EF operations_execution_payload tests (real crypto), clippy clean (full workspace including tests).

**Conclusion**: Phase 5 complete. Gloas test quality is strong — comprehensive coverage, specific assertions, deterministic execution. No actionable gaps found that justify new tests. The codebase has ~600+ Gloas-specific tests across all layers.
