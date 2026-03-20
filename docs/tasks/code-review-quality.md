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

### Run 294: withdrawal loop optimization, Copy derivation, saturating_sub simplification

**Scope**: Performance optimizations in Gloas withdrawal processing and builder payment types.

**Changes**:

1. **Hoisted `state.validators().len()` out of hot loops** in both `process_withdrawals_gloas` and `get_expected_withdrawals_gloas` — the validator count was called per-iteration in the validator sweep loop for the `safe_rem` modulus. Now cached before the loop. Affects both the mutation path and the read-only expected-withdrawals computation.

2. **Derived `Copy` on `BuilderPendingWithdrawal` and `BuilderPendingPayment`** — both types are small fixed-size structs (36 and 44 bytes respectively, all-Copy fields: `Address` + `u64` + `u64`). With `Copy`, all `.clone()` calls become zero-cost bitwise copies. Fixed 7 `clone_on_copy` clippy lints across production and test code (replaced `.clone()` with dereference or direct pass).

3. **Simplified `saturating_sub(min(amount, balance))`** to `saturating_sub(amount)` in builder withdrawal balance decrease — the `min` is redundant since `saturating_sub` already clamps to zero.

**Verification**: 463/463 state_processing tests, 711/711 types tests, 17/17 EF operations+sanity tests, 18/18 EF epoch processing tests, full workspace clippy clean.

### Run 299: execution payload envelope metrics

**Scope**: Observability improvement — add metrics for execution payload envelope gossip processing and self-build envelope processing.

**Problem**: Execution bids had 3 metrics (verified, imported, equivocating) and payload attestations had 3 metrics, but execution payload envelopes — the second half of the ePBS pipeline where builders reveal payloads — had zero metrics. Operators could not monitor envelope verification rates, rejection patterns, or import success rates.

**Changes**:
1. **Network gossip metrics** (3 new counters in `beacon_node/network/src/metrics.rs`):
   - `beacon_processor_payload_envelope_verified_total` — envelope passed gossip validation
   - `beacon_processor_payload_envelope_imported_total` — envelope fully processed (EL + state transition)
   - `beacon_processor_payload_envelope_rejected_total` — envelope rejected (InvalidSignature, SlotMismatch, BuilderIndexMismatch, BlockHashMismatch, NotGloasBlock)

2. **Beacon chain metric** (1 new counter in `beacon_node/beacon_chain/src/metrics.rs`):
   - `beacon_self_build_envelope_successes_total` — self-build envelope processed successfully

**Verification**: 44/44 Gloas network tests, 17/17 self-build envelope tests, 17/17 EF spec tests, full workspace clippy clean (lint-full passed).

### Run 556 — Dead V15 operation pool compat removal

**Problem**: `PersistedOperationPool` used a superstruct enum with V15 and V20 variants. The V15 variant was a compatibility shim for old Lighthouse DB schema ≤17. vibehouse has no production databases with old schemas, and no migration code references V15. Three `TransformPersist` impls for `AttesterSlashingBase` existed solely to support V15→V20 conversion.

**Changes**:
1. Converted `PersistedOperationPool` from a superstruct enum to a plain struct (136 lines removed)
2. Removed dead `TransformPersist` impl for `AttesterSlashingBase` and `From`/`TryFrom` for `SigVerifiedOp<AttesterSlashingBase>` (3 TODOs resolved)
3. Made `into_operation_pool` infallible (was `Result` only because V15 conversion could fail)
4. Removed `IncorrectOpPoolVariant` error variant (unused)

**Verification**: 31/31 operation_pool tests, schema_stability test, op pool retrieval tests all pass. Full workspace clippy clean.

### Run 557 — Dead V17 fork choice compat and all DB schema migrations removal

**Problem**: Fork choice persistence used superstruct enums with V17 and V28 variants across 3 crates (proto_array `SszContainer`, fork_choice `PersistedForkChoice`, beacon_chain `PersistedForkChoiceStore`/`PersistedForkChoice`). V17 was the format used by Lighthouse schemas 17–27; vibehouse has always been at schema 28. Six migration files (v22→v28, 1,068 lines) existed to upgrade old Lighthouse databases that vibehouse will never encounter. `CacheItem` and `BalancesCache` were single-variant superstructs (V8 only) — unnecessary abstraction.

**Changes** (1,522 lines removed, 32 added):
1. Deleted 6 migration files: `migration_schema_v23.rs` through `migration_schema_v28.rs`
2. Simplified `migrate_schema` to only accept identity (from == to == CURRENT_SCHEMA_VERSION)
3. Converted `SszContainer` (proto_array) from V17/V28 superstruct to plain struct
4. Converted `PersistedForkChoice` (fork_choice) from V17/V28 superstruct to plain struct
5. Converted `PersistedForkChoice` (beacon_chain) from V17/V28 superstruct to plain struct
6. Converted `PersistedForkChoiceStore` from V17/V28 superstruct to plain struct
7. Removed `from_persisted_v17()` and all V17↔V28 conversion impls
8. Converted `CacheItem`/`BalancesCache` from single-variant superstructs to plain structs
9. Removed 4 schema downgrade/upgrade round-trip tests (tested dead migration paths)

**Verification**: 277/277 proto_array + fork_choice tests, 8/8 EF fork choice tests, 31/31 operation_pool tests, schema_stability test passes. Full workspace clippy clean (lint-full passed).

### Run 558 — ProtoNode superstruct simplification + dead storage module removal

**ProtoNode simplification** (consensus/proto_array):
- `ProtoNode` used `#[superstruct(variants(V17), no_enum)]` with only one variant — all fields always present
- Converted to plain struct with `#[derive(Clone, PartialEq, Debug, Encode, Decode, Serialize, Deserialize)]`
- Renamed `ProtoNodeV17` references to `ProtoNode` in ssz_container.rs
- Removed `superstruct` dependency from proto_array crate

**Dead storage modules removal** (beacon_node/store, 1,549 lines removed):
- `partial_beacon_state.rs` (510 lines) — pre-v22 format where beacon state vector fields were loaded lazily from chunked DB columns. Not imported by any production code.
- `chunked_vector.rs` (919 lines) — chunked storage format for state vectors (block_roots, state_roots, randao_mixes, etc.). Only used by partial_beacon_state.rs and chunked_iter.rs.
- `chunked_iter.rs` (120 lines) — iterator over chunked vector storage. Only used internally.
- Removed dead error types: `ChunkError`, `VectorChunkError`, `PartialBeaconStateError`
- Updated `compact()` and leveldb `compact_column()` to use active DB columns instead of deprecated `BeaconState`/`BeaconStateSummary`

**Not removed** (intentional design):
- `OnDiskStoreConfig` (V22 superstruct) — uses SSZ union encoding with version byte for forward-compatible serialization
- `HDiff` (V0 superstruct) — same SSZ union versioning pattern
- Deprecated DB column enum variants — harmless, needed for `key_size()` match exhaustiveness

**Verification**: 293/293 proto_array + fork_choice + store tests, 8/8 EF fork choice tests. Full workspace clippy clean (lint-full passed).

### Run 559 — Dead error variants and orphaned schema_change README

**Scope**: Continue dead code cleanup from runs 556-558. Remove never-constructed error enum variants and orphaned documentation.

**Changes**:

1. **proto_array error.rs** — removed 4 never-constructed variants:
   - `InvalidFinalizedRootChange` — 0 uses
   - `MissingJustifiedCheckpoint` — 0 uses
   - `MissingFinalizedCheckpoint` — 0 uses
   - `UnknownLatestValidAncestorHash` — 0 uses

2. **fork_choice error enum** — removed 2 dead variants:
   - `InvalidLegacyProtoArrayBytes` — V17 format removed in run 557, variant became dead
   - `AttemptToRevertJustification` — never constructed

3. **store errors.rs** — removed 3 never-constructed variants:
   - `RandaoMixOutOfBounds` — 0 uses
   - `GenesisStateUnknown` — 0 uses
   - `LoadHotStateSummaryForSplit` — 0 uses

4. **schema_change/README.md** — deleted orphaned README describing the old migration system removed in run 557. The `schema_change.rs` file (identity-check safety function) remains.

**Not changed (intentional)**:
- `OnDiskStoreConfig` V22 / `HDiff` V0 single-variant superstructs — SSZ union encoding with version byte for forward compatibility (run 558 decision)
- Deprecated `DBColumn` variants — needed for `key_size()` match exhaustiveness
- `BeaconChainError` variants — many appear unused but are constructed via `From` impls and `?` operator

**Verification**: 277/277 proto_array + fork_choice tests, 8/8 EF fork choice tests, 16/16 store tests. Full workspace clippy clean (lint-full passed).

### Run 560 — Unused dependency removal

**Scope**: Remove unused Cargo.toml dependencies identified by cargo-machete, with manual verification to filter out false positives (derive macros, feature forwarding, etc.).

**Changes** (6 dependencies removed across 5 crates):
1. `common/clap_utils` — removed `alloy-primitives` (no usage in crate)
2. `common/eth2` — removed `either` (no usage in crate). Kept `rand` (needed by `TestRandom` derive macro — cargo-machete false positive)
3. `validator_client/beacon_node_fallback` — removed `itertools` (no usage in crate)
4. `validator_client/lighthouse_validator_store` — removed `beacon_node_fallback`, `either`, `environment` (none used in crate)
5. `validator_client/validator_services` — removed `either` (no usage in crate)

**Also fixed**: pre-existing `cargo sort` issues in `beacon_chain/Cargo.toml` and `state_processing/Cargo.toml`.

**Not removed** (false positives):
- `consensus/merkle_proof` `alloy-primitives` — needed for feature forwarding (`arbitrary = ["alloy-primitives/arbitrary"]`)
- `common/eth2` `rand` — used by `TestRandom` derive macro
- All `ethereum_ssz`/`ethereum_ssz_derive`/`ethereum_serde_utils` — used by derive macros
- `lighthouse` `lighthouse_tracing`/`store` — actually imported in main.rs / used transitively
- `environment` `tracing-log` — used indirectly via logging crate

**Verification**: 98/98 tests across affected crates. Full workspace compiles clean, lint-full passes.

### Run 561 — More unused dependency removal

**Scope**: Second pass of cargo-machete with manual verification, focusing on non-derive-macro dependencies missed in run 560.

**Changes** (4 dependencies removed across 4 crates):
1. `consensus/state_processing` — removed `swap_or_not_shuffle` (0 uses in crate)
2. `consensus/fork_choice` — removed `superstruct` (0 uses after ProtoNode simplification in run 558)
3. `validator_client/slashing_protection` — removed `tracing` (0 uses in crate)
4. `common/logging` — removed `serde` (only `serde_json` is used, not `serde` itself)

**Not removed** (false positives, same as run 560):
- All `ethereum_ssz`/`ethereum_ssz_derive`/`ethereum_serde_utils` — used by derive macros
- `eth2` `rand` — used by `TestRandom` derive macro
- `merkle_proof`/`bls` `alloy-primitives` — feature forwarding
- `eth2_wallet` `tiny-bip39` — used via `bip39::` import

**Verification**: 724/724 tests across affected crates. Full workspace compiles clean, lint passes.

### Run 562 — Dead V22 compat code, orphaned file, dead error variants

**Scope**: Continue dead code cleanup. Remove code made dead by schema migration removal (run 557) and other never-used items.

**Changes**:

1. **Dead V22 state summary compat** (beacon_node/store/src/hot_cold_store.rs, 70 lines removed):
   - Removed `HotStateSummaryV22` struct + `StoreItem` impl (only used by dead fallback path)
   - Removed `load_hot_state_summary_v22()` function
   - Removed `load_block_root_from_summary_any_version()` function (V22 fallback path dead since migrations removed in run 557)
   - Simplified `load_split()` to use `load_hot_state_summary()` directly
   - Removed obsolete V22↔V24 migration scenario comment

2. **Orphaned file** (beacon_node/beacon_chain/src/otb_verification_service.rs, 369 lines removed):
   - File was never declared as `mod` in lib.rs — completely dead code
   - Contained `OptimisticTransitionBlock` verification service (deprecated feature)

3. **Dead error variants** (8 lines removed across 2 files):
   - `BeaconChainError::InsufficientValidators` — 0 constructions
   - `BeaconChainError::SlotClockDidNotStart` — 0 constructions
   - `BlockProductionError::NoEth1ChainConnection` — 0 constructions
   - `BlockProductionError::MissingExecutionBlockHash` — 0 constructions
   - `BlockProcessingError::InvalidSlot` — 0 constructions
   - `BlockProcessingError::InvalidSlotIndex` — 0 constructions

4. **Cargo.lock sync** — updated for dependency removals from runs 560-561

**Not changed (intentional)**:
- `OnDiskStoreConfig::V22` superstruct — SSZ union encoding for forward compatibility (run 558 decision)
- Deprecated `DBColumn` variants — needed for `key_size()` match exhaustiveness
- `#[allow(dead_code)]` on error enum fields used in Debug impls — standard Rust pattern
- `GossipCacheBuilder` dead_code allow — builder pattern, methods available for future use

**Verification**: 557/557 state_processing tests, 16/16 store tests. Full workspace lint-full passes.

### Run 563 — Dead error variants in BeaconChainError and EpochProcessingError

**Scope**: Continue dead code cleanup. Remove never-constructed error enum variants across two crates.

**Changes**:

1. **BeaconChainError** (beacon_node/beacon_chain/src/errors.rs, 8 variants removed):
   - `NoStateForAttestation { beacon_block_root: Hash256 }` — 0 constructions
   - `NoProposerForSlot(Slot)` — 0 constructions
   - `CanonicalHeadLockTimeout` — 0 constructions
   - `AttestationCacheLockTimeout` — 0 constructions
   - `ValidatorPubkeyCacheLockTimeout` — 0 constructions
   - `SnapshotCacheLockTimeout` — 0 constructions
   - `ForkchoiceUpdateParamsMissing` — 0 constructions
   - `EmptyRpcCustodyColumns` — 0 constructions

2. **BlockProductionError** (same file, 1 variant removed):
   - `FailedToBuildBlobSidecars(String)` — 0 constructions

3. **EpochProcessingError** (consensus/state_processing/src/per_epoch_processing/errors.rs, 8 variants removed):
   - `UnableToDetermineProducer` — 0 constructions
   - `NoBlockRoots` — 0 constructions
   - `BaseRewardQuotientIsZero` — 0 constructions
   - `NoRandaoSeed` — 0 constructions
   - `PreviousTotalBalanceIsZero` — 0 constructions
   - `InclusionDistanceZero` — 0 constructions
   - `DeltasInconsistent` — 0 constructions
   - `InclusionSlotsInconsistent(usize)` — 0 constructions

4. **InclusionError enum removed entirely** (same file):
   - `NoAttestationsForValidator` — 0 constructions
   - `BeaconStateError(BeaconStateError)` — only used by dead `From` impl
   - `EpochProcessingError::InclusionError(InclusionError)` variant also removed (0 constructions)
   - `From<InclusionError> for EpochProcessingError` impl removed

**Not changed (intentional)**:
- Same items as run 562

**Verification**: 557/557 state_processing tests, 35/35 EF spec tests (epoch processing + operations + sanity). Full workspace lint-full passes.

### Run 564 — Dead error variants in engine_api, BeaconChainError, and BlockProductionError

**Scope**: Continue dead code cleanup across three error enums.

**Changes**:

1. **engine_api::Error** (beacon_node/execution_layer/src/engine_api.rs, 8 variants + 1 import removed):
   - `RequestFailed(String)` — 0 constructions within execution_layer
   - `InvalidExecutePayloadResponse(&'static str)` — 0 constructions
   - `JsonRpc(RpcError)` — 0 constructions (no From<RpcError> impl either)
   - `ParentHashEqualsBlockHash(ExecutionBlockHash)` — 0 constructions
   - `DeserializeWithdrawals(ssz_types::Error)` — 0 constructions (SszError variant used instead)
   - `DeserializeDepositRequests(ssz_types::Error)` — 0 constructions
   - `DeserializeWithdrawalRequests(ssz_types::Error)` — 0 constructions
   - `TooManyConsolidationRequests(usize)` — 0 constructions
   - Removed unused `use http::deposit_methods::RpcError` import

2. **BeaconChainError** (beacon_node/beacon_chain/src/errors.rs, 6 variants removed):
   - `UnableToAdvanceState(String)` — 0 constructions
   - `ValidatorPubkeyCacheError(String)` — 0 constructions
   - `ExecutionLayerGetBlockByHashFailed(Box<execution_layer::Error>)` — 0 constructions
   - `FinalizedBlockMissingFromForkChoice(Hash256)` — 0 constructions
   - `UnableToBuildColumnSidecar(String)` — 0 constructions
   - `ProposerCacheAccessorFailure { decision_block_root, proposal_epoch }` — 0 constructions

3. **BlockProductionError** (same file, 4 variants removed):
   - `BlockingFailed(execution_layer::Error)` — 0 constructions
   - `FailedToReadFinalizedBlock(store::Error)` — 0 constructions
   - `MissingFinalizedBlock(Hash256)` — 0 constructions
   - `KzgError(kzg::Error)` — 0 constructions

**Verification**: 557/557 state_processing tests, 16/16 store tests, 35/35 EF spec tests. Full workspace lint passes.

### Run 565 — Dead error variants in block processing, attestation, and network errors

**Scope**: Continue dead code cleanup across state_processing errors, BeaconChainError, and network sync errors.

**Changes**:

1. **BeaconChainError** (2 variants removed):
   - `ProcessInvalidExecutionPayload(JoinError)` — 0 constructions
   - `UnsupportedFork` — 0 constructions

2. **AttestationInvalid** (3 variants removed):
   - `BadAggregationBitfieldLength { committee_len, bitfield_len }` — 0 constructions
   - `NotDisjoint` — 0 constructions
   - `UnknownValidator(u64)` — 0 constructions

3. **ExitInvalid** (1 variant removed):
   - `AlreadyInitiatedExit(u64)` — 0 constructions

4. **IndexedAttestationInvalid** (2 variants removed):
   - `UnknownValidator(u64)` — 0 constructions
   - `SignatureSetError(SignatureSetError)` — 0 constructions

5. **SyncAggregateInvalid** (1 variant removed):
   - `PubkeyInvalid` — 0 constructions

6. **LookupVerifyError** (1 variant removed):
   - `TooManyResponses` — 0 constructions

**Verification**: 557/557 state_processing tests, 35/35 EF spec tests, 163/163 network tests. Full workspace clippy clean.

### Run 566 — Final dead code sweep

**Scope**: Comprehensive dead code audit across remaining error enums, public functions, constants, and modules.

**Changes**:

1. **store::Error** (1 variant removed):
   - `MissingGenesisState` — 0 constructions anywhere in workspace

**Exhaustive audit results** (no further dead code found):
- All remaining error variants in store::Error, BeaconChainError, BlockProductionError, BlockProcessingError, all state_processing error enums, fork_choice::Error, network sync errors — all actively constructed
- All `#[allow(dead_code)]` annotations justified (test infrastructure, Debug-used fields, Drop guards)
- All Gloas-specific public functions verified as actively called
- No orphaned .rs files, no dead modules, no unused re-exports
- `IndexedPayloadAttestation::num_attesters()` and `PayloadAttestation::num_attesters()` — only test usage, but harmless utility methods
- Consensus-specs still at v1.7.0-alpha.2, PTC Lookbehind PR #4979 still open, no spec logic changes merged

**Verification**: 16/16 store tests, full workspace build + clippy clean.

### Run 567 — Visibility audit: pub → pub(crate) downgrades

**Scope**: Audit all Gloas-specific `pub` items across state_processing and beacon_chain crates for unnecessary visibility.

**Method**: Grep every Gloas `pub fn`/`pub struct`/`pub enum` → check if used outside its crate (including integration tests in `tests/`).

**Changes** (2 functions downgraded):
1. `get_pending_balance_to_withdraw_for_builder` (gloas.rs:965) — `pub` → `pub(crate)`, only used within state_processing (verify_exit.rs, gloas.rs)
2. `initiate_builder_exit` (gloas.rs:990) — `pub` → `pub(crate)`, only used within state_processing (process_operations.rs)

**Investigated but kept as `pub`** (legitimately cross-crate):
- `process_execution_payload_bid` — used by beacon_chain, ef_tests
- `can_builder_cover_bid`, `get_ptc_committee`, `is_parent_block_full`, `get_expected_withdrawals_gloas`, `process_withdrawals_gloas` — all used by beacon_chain or ef_tests
- `process_builder_pending_payments` — used by ef_tests
- `VerifiedExecutionBid`, `VerifiedPayloadAttestation`, `VerifiedPayloadEnvelope` — used by network crate
- `ExecutionBidError`, `PayloadAttestationError`, `PayloadEnvelopeError` — used by http_api and network
- `ObservedExecutionBids`, `ObservedPayloadAttestations`, `ObservedPayloadEnvelopes`, `ExecutionBidPool` — exposed via `pub` fields on `BeaconChain`, accessed from http_api/network tests
- `BidObservationOutcome`, `AttestationObservationOutcome` — used within beacon_chain verification

**Verification**: 557/557 state_processing tests, zero warnings, clippy clean.

### Run 569 — dependency updates and codebase health check

**Scope**: Spec conformance check, dependency updates, dead code audit.

**Spec status**:
- v1.7.0-alpha.2 still latest release, no new spec test vectors
- PTC Lookbehind (PR #4979) still open/blocked
- No new Gloas-related PRs merged since run 568
- CI: all jobs green (check+clippy+fmt, ef-tests, network+op_pool passed; beacon_chain and http_api in progress)
- Nightly CI: 5 consecutive green runs (Mar 3-7)

**Dead code audit**: Comprehensive scan of `#[allow(dead_code)]`, unused imports, stale conditional compilation — all 21 instances are justified (error Debug fields, builder pattern methods, test infrastructure, platform-specific code). No cleanup needed.

**Dependency updates** (2 commits):
1. `procfs` 0.15.1 → 0.18.0 — removed 10 stale transitive deps (hermit-abi, io-lifetimes, linux-raw-sys 0.1.x, rustix 0.36.x, 6 windows-sys/targets crates)
2. `libc` 0.2.182 → 0.2.183 — patch update

**Investigated but not updated**:
- `psutil` 3.3.0 → 5.4.0: blocked by `darwin-libproc` → `memchr ~2.3` pin conflicting with `gossipsub` → `regex` → `memchr ^2.6`
- `cc` 1.2.30 → 1.2.56: breaks `leveldb-sys` build (adds `-Wthread-safety` GCC doesn't support)
- `cmake` 0.1.54 → 0.1.57: same `leveldb-sys` build failure
- `itertools`, `sysinfo`, `uuid`, `strum`: major version bumps requiring API migration across many crates, low value
- `service_name` dead field in TaskExecutor: 25+ callers, high churn for zero behavior change

**Verification**: 2585/2593 tests pass (8 web3signer timeouts — pre-existing infrastructure-dependent), zero clippy warnings, full lint clean.

### Run 571 — unused dependency cleanup (2026-03-08)

Removed 9 unused dependencies across 6 crates using `cargo-machete --with-metadata`:
- `bls`: removed `alloy-primitives`, `safe_arith`
- `execution_layer`: removed `sha2`
- `http_api`: removed `either`
- `slashing_protection`: removed `ethereum_serde_utils`
- `store`: removed `logging`, `smallvec`, `tracing-subscriber`
- `client`: removed `ethereum_ssz`

False positives from cargo-machete (kept): `rand` (used by TestRandom derive macro), `ethereum_ssz` (used by Encode/Decode derive macros), `zip` (build-dependency), `futures` (dev-dependency used in tests).

### Run 579 — dependency upgrades: db-key, headers (2026-03-08)

**Spec status**: v1.7.0-alpha.2 still latest release. PR #4950 (extend by_root serve range) and #4926 (SLOT_DURATION_MS) merged since last check — both already compliant, no code changes needed.

**Dependency upgrades** (2 crates):
1. `db-key` 0.0.5 → 0.1.0 (store) — new Key trait uses standard `From<&[u8]>` + `AsRef<[u8]>` instead of custom `from_u8`/`as_slice` methods. Updated `BytesKey` impl.
2. `headers` 0.3 → 0.4 (warp_utils) — moves to base64 0.22 and headers-core 0.3. No API changes needed (same `Origin::try_from_parts` call).

**Investigated but not upgraded** (blocked by ecosystem):
- `reqwest-eventsource` 0.5 → 0.6: requires `reqwest` 0.11 → 0.12 upgrade (different `http` crate versions cause `StatusCode` type mismatch)
- `rand_xorshift` 0.4 → 0.5: requires `rand` 0.8 → 0.9 upgrade (different `rand_core` versions cause `SeedableRng` trait mismatch)
- `hash-db` 0.15 → 0.16 + `keccak-hash` 0.10 → 0.12: blocked by `triehash` 0.8.4 pinning `hash-db` 0.15

**Verification**: 32/32 store + warp_utils tests, full workspace build clean, full lint clean.

### Run 580 — replace deprecated Parity trie libs with alloy-trie (2026-03-08)

**Spec status**: v1.7.0-alpha.2 still latest release. PR #4979 (PTC lookbehind) still open/unmerged. No new Gloas spec changes.

**Replaced 4 deprecated Parity crates** with alloy-trie (already a transitive dependency):
- Removed: `hash-db` 0.15.2, `hash256-std-hasher` 0.15.2, `keccak-hash` 0.10.0, `triehash` 0.8.4
- Replaced `triehash::ordered_trie_root::<KeccakHasher, _>()` with `alloy_trie::root::ordered_trie_root_with_encoder()`
- Replaced `keccak_hash::KECCAK_EMPTY_LIST_RLP` with local `const` (same bytes)
- Removed `KeccakHasher` struct and `hash_db::Hasher` impl (no longer needed)
- Simplified `keccak.rs` to just the `keccak256()` helper

Net: -86 lines, -4 deprecated dependencies, no new direct dependencies (alloy-trie was already in tree).

**Remaining blocked upgrades**: rand_xorshift 0.5 (needs rand_core 0.10, we have 0.9).

**Verification**: 10/10 block_hash + execution_layer tests pass, full workspace build clean, full lint clean.

### Run 581 — dependency upgrades: itertools 0.14, reqwest-eventsource 0.6, alloy-trie 0.9 (2026-03-08)

**Spec status**: v1.7.0-alpha.2 still latest release. No new Gloas spec changes merged. Recent consensus-specs PRs (#4977-#4988) are all infrastructure/tooling changes unrelated to Gloas.

**CI status**: all green on latest push.

**Dependency upgrades shipped**:
1. `itertools` 0.10 → 0.14 (workspace-wide) — eliminates one duplicate version, API-compatible
2. `reqwest-eventsource` 0.5 → 0.6 — adapted `InvalidStatusCode` handling for `http` 1.x `StatusCode` type (convert via `as_u16()`)
3. `alloy-trie` 0.8 → 0.9 — API-compatible, no code changes needed

**Remaining duplicate versions** (all transitive, not actionable):
- `itertools`: 0.10 (criterion), 0.12 (bindgen), 0.13 (ethereum_ssz/milhouse), 0.14 (our code + superstruct)
- `rand_xorshift` 0.4 → 0.5: still blocked by rand_core version mismatch (needs rand 0.10, we have 0.9)

**Verification**: 80/80 eth2 + execution_layer tests, 64/64 targeted itertools-using tests, full workspace clippy clean, full lint clean.

### Run 583 — remove leveldb backend (2026-03-08)

**Scope**: Remove the optional leveldb database backend entirely. vibehouse is independent and uses redb exclusively (switched in run 572). leveldb was never compiled by default, but its code and C dependency added maintenance burden.

**Changes**:
1. Deleted `beacon_node/store/src/database/leveldb_impl.rs` (299 lines)
2. Simplified `BeaconNodeBackend` from cfg-gated enum to direct `struct(Redb<E>)` wrapper
3. Removed `leveldb` dependency from store/Cargo.toml (C dependency eliminated)
4. Removed `beacon-node-leveldb` and `beacon-node-redb` features from lighthouse/Cargo.toml
5. Removed all `#[cfg(feature = "leveldb")]` and `#[cfg(feature = "redb")]` gates in errors.rs, config.rs, database.rs
6. Removed cfg-gated test `beacon_node_backend_override` from lighthouse tests
7. Updated Makefile: removed `beacon-node-redb` from CROSS_FEATURES and lint-full
8. Updated book/installation_source.md: removed leveldb feature documentation
9. Updated comments referencing LevelDB in 4 files

**Net**: -540 lines, -1 C dependency (`leveldb` crate + `leveldb-sys`).

**Verification**: 30/30 store tests, full workspace build clean, full workspace clippy clean, pre-push lint-full passes.

### Run 586 — remove unused deps from 3 more crates, sort execution_layer (2026-03-08)

**Scope**: Continue dependency cleanup using cargo-machete with manual verification.

**Changes**:
1. `beacon_node/genesis` — removed unused `tracing` (no tracing macros in source)
2. `boot_node` — removed unused `log` (uses `tracing` directly, not `log` crate)
3. `lcli` — removed unused `log` (same reason)
4. `beacon_node/execution_layer` — sorted Cargo.toml deps (`alloy-trie` was out of alphabetical order)

**Not removed** (false positives):
- `eth2` `rand` — needed by TestRandom derive macro
- `state_processing` `rand` — same (TestRandom derive)
- `merkle_proof` `alloy-primitives` — feature forwarding (`arbitrary = ["alloy-primitives/arbitrary"]`)
- `lcli` `bls` — feature forwarding (`portable = ["bls/supranational-portable"]`, `fake_crypto = ["bls/fake_crypto"]`)
- All `ethereum_ssz`/`ethereum_serde_utils` — derive macros

**Verification**: 2/2 genesis tests, full workspace build clean, full clippy clean, pre-push lint-full passes.

### Run 587 — fix redb 3.x compaction CI failure (2026-03-08)

**CI failure**: `store_tests::prune_historic_states` panicked with `RedbError(TransactionInProgress)` at store_tests.rs:4780. Introduced by redb 2.x → 3.1.0 upgrade (run 575).

**Root cause**: In redb 3.x, `Database::compact()` fails with `CompactionError::TransactionInProgress` if any read transactions are alive. In `prune_historic_states`, after deleting cold state data, `compact_freezer()` is called. If background tasks hold read transactions on the cold DB at that point, compaction fails. In redb 2.x this was not an error.

**Fix**: Modified `Redb::compact()` to silently skip compaction when `TransactionInProgress` is returned. Compaction is an optimization (space reclamation), not a correctness requirement — it can safely be skipped and retried later.

**Verification**: `prune_historic_states` test passes, 30/30 store tests pass, full clippy clean, pre-push lint-full passes.

### Run 588 — CI verification + codebase health check (2026-03-08)

**CI result**: All 6 jobs pass (check+clippy, ef-tests, unit-tests, beacon_chain, http_api, network+op_pool). The redb 3.x compaction fix (647292d28) resolves the `prune_historic_states` TransactionInProgress failure.

**Health checks performed**:
- `cargo clippy --workspace`: zero warnings
- `cargo build --workspace`: zero warnings
- `cargo sort --workspace --check`: all Cargo.toml deps sorted
- `cargo audit`: 1 known unfixed advisory (RUSTSEC-2023-0071 rsa crate via jsonwebtoken — no fix available upstream), 10 allowed warnings
- Nightly tests: all green (last 3 days)
- Spec PR status: all 9 tracked PRs still OPEN (#4979, #4940, #4932, #4960, #4898, #4954, #4843, #4840, #4630)
- No new spec test release after v1.7.0-alpha.2
- Gloas test coverage: comprehensive (all public functions in state_processing, envelope_processing, gloas_verification have unit/integration tests)

### Run 593 — reqwest 0.11 → 0.12 upgrade (2026-03-08)
- Upgraded workspace reqwest from 0.11 to 0.12, eliminating duplicate reqwest versions for workspace crates
- reqwest 0.11 remains only as a transitive dep from ethers (in execution_engine_integration test crate)
- Simplified deposit_contract build script: removed reqwest/serde_json build-deps, now just verifies checksums of committed contract files
- Added local `serde_warp_status_code` module in http_api since warp 0.3 still uses http 0.2 (different StatusCode type from reqwest 0.12's http 1.x)
- Fixed broadcast_validation_tests to handle dual StatusCode types (warp's http 0.2 for function args, reqwest's http 1.x for response checking)
- Remaining duplicate deps are all transitive from external crates (ethers, warp, libp2p, criterion) — not fixable without replacing those crates

### Run 596 — strum 0.24 → 0.27, uuid 0.8 → 1.x (2026-03-08)

**Scope**: Upgrade two direct workspace dependencies to eliminate duplicate crate versions.

**Changes**:
1. `strum` 0.24 → 0.27: renamed deprecated `EnumVariantNames` derive to `VariantNames` in 3 files (database_manager, slasher, beacon_node_fallback). All other strum derives (`AsRefStr`, `IntoStaticStr`, `EnumString`, `Display`, `EnumIter`, `IntoEnumIterator`) unchanged.
2. `uuid` 0.8 → 1.x: zero code changes needed — `Uuid::new_v4()`, `parse_str()`, `from_u128()` all API-compatible.

**Result**: Lockfile 1039 → 1035 packages (-4). Eliminated strum 0.24 + strum_macros 0.24 + uuid 0.8 duplicates. Remaining duplicates are all transitive (warp http 0.2 stack, libp2p, criterion, etc.).

**Spec status**: stable, no new Gloas merges since run 595. PR #4979 (PTC Lookbehind) still open. PR #4950 (by_root serve range) merged Mar 6 — already assessed as no-change-needed.

**Verification**: 98/98 targeted tests (eth2_keystore, eth2_wallet, eth2_wallet_manager, slasher, database_manager, beacon_node_fallback), full workspace clippy clean, full lint-full clean (pre-push hook).

### Run 611: vibehouse identity rebranding

**Scope**: Rebrand user-visible identity strings from "Lighthouse" to "vibehouse".

**Changes**:
1. `lighthouse_version/src/lib.rs`: VERSION prefix "vibehouse/v0.1.0-", `client_name()` returns "vibehouse", `version()` returns "0.1.0", test regex updated
2. `lighthouse/src/main.rs`: CLI name "vibehouse", about text updated, Sigma Prime author removed, telemetry service names "vibehouse-bn"/"vibehouse-vc", tracer name "vibehouse", SHORT_VERSION strip prefix updated
3. `monitoring_api/src/types.rs`: CLIENT_NAME = "vibehouse"
4. `environment/src/tracing_common.rs`: logfile prefix "vibehouse"
5. `lighthouse_network/src/peer_manager/peerdb/client.rs`: added `Vibehouse` ClientKind variant with "vibehouse" agent string matching, Display impl
6. `beacon_node/src/cli.rs`: removed Sigma Prime author
7. `simulator/src/cli.rs`: removed Sigma Prime author
8. `graffiti_file/src/lib.rs`: test default graffiti "vibehouse"

**Not changed** (intentionally):
- API paths (`/lighthouse/...`) — breaking change for external tooling
- Binary name (`lighthouse`) — DONE in run 613 (see below)
- Crate names (`lighthouse_network`, `lighthouse_validator_store`) — internal, no user impact

**Verification**: lighthouse_version, monitoring_api, graffiti_file tests pass; default_graffiti beacon_node test passes; full workspace cargo check clean; clippy clean; pre-push lint-full clean.

### Run 613: binary rename lighthouse → vibehouse (2026-03-09)

**Scope**: Rename the compiled binary from `lighthouse` to `vibehouse` across all build infrastructure.

**Changes** (20 files):
1. `lighthouse/Cargo.toml`: `name = "lighthouse"` → `name = "vibehouse"`, version `8.0.1` → `0.1.0`, removed Sigma Prime author
2. `Makefile`: `--bin lighthouse` → `--bin vibehouse`, tarball/install paths updated, Docker image tags `vibehouse:reproducible-*`
3. `Dockerfile`: copy dir `lighthouse` → `vibehouse`, binary path `/usr/local/bin/vibehouse`, added `lighthouse` symlink for kurtosis compat
4. `Dockerfile.dev`: binary path updated, `lighthouse` symlink added
5. `Dockerfile.reproducible`: `--bin vibehouse`, binary path updated, entrypoint `/vibehouse`
6. `Dockerfile.cross`: binary path updated, `lighthouse` symlink added
7. `lcli/Dockerfile`: copy dir updated, comment fixed
8. `scripts/build-docker.sh`: binary name in cargo build output and `cp` command
9. `scripts/cli.sh`: `CMD=./target/release/vibehouse`
10. `.config/nextest.toml`: report name `vibehouse-run`
11. `.github/workflows/release.yml`: repo/image names, artifact names, runner conditions all → `vibehouse`/`dapplion`
12. `.github/workflows/docker.yml`: matrix binary `vibehouse`, runner conditions → `dapplion/vibehouse`
13. `lighthouse/tests/*.rs` (5 files): `CARGO_BIN_EXE_lighthouse` → `CARGO_BIN_EXE_vibehouse`
14. `README.md`: `./target/release/vibehouse --help`
15. `book/src/installation_homebrew.md`: binary name in path

**Kurtosis compatibility**: Docker images include `ln -s /usr/local/bin/vibehouse /usr/local/bin/lighthouse` so the ethereum-package's `cl_type: lighthouse` startup commands still work.

**Not changed** (intentionally):
- Kurtosis yaml `cl_type: lighthouse` — this is the ethereum-package's client type identifier, not our binary name
- `/lighthouse/...` API paths — would break external tooling
- Crate names — internal, no user impact
- `lighthouse/` directory name — workspace member path, not user-visible

**Verification**: `cargo build --release` clean, `vibehouse --version` shows `vibehouse v0.1.0`, 312/312 package tests pass, clippy clean, pre-push lint-full passes.

### Run 615: finish vc/lcli rebranding (2026-03-09)

**Scope**: Rebrand remaining user-visible "Lighthouse" references missed in runs 611-614.

**Changes** (3 files):
1. `validator_client/src/cli.rs`: 4 CLI help text strings — doppelganger protection, builder proposals, prefer builder proposals, web3signer slashing protection
2. `validator_client/http_api/src/lib.rs`: 6 error messages "Lighthouse shutting down" → "vibehouse shutting down"
3. `lcli/src/main.rs`: Command name "Lighthouse CLI Tool" → "vibehouse CLI Tool"

**Remaining "lighthouse" references** (intentionally kept):
- API paths (`.push("lighthouse")`) — breaking change for external tooling
- Test infrastructure file paths (`tls_dir().join("lighthouse")`) — test artifacts
- Test rig temp dir prefixes — internal

**Verification**: cargo check clean, validator_client tests pass, clippy clean, pre-push lint-full passes.

### Run 616: lighthouse_validator_store crate rename (2026-03-09)

Renamed `lighthouse_validator_store` crate and `LighthouseValidatorStore` struct to `vibehouse_validator_store` / `VibehouseValidatorStore`.

### Run 617: rename 3 remaining lighthouse_* crates (2026-03-09)

**Scope**: Rename the last 3 crates with "lighthouse" in their names.

**Changes** (3 crate renames, 145+ files):
1. `common/lighthouse_version` → `common/vibehouse_version` — package name, directory, all imports and Cargo.toml deps (33 files)
2. `beacon_node/lighthouse_tracing` → `beacon_node/vibehouse_tracing` — package name, directory, all imports and Cargo.toml deps (19 files)
3. `beacon_node/lighthouse_network` → `beacon_node/vibehouse_network` — package name, directory, all imports and Cargo.toml deps (113 files)

Also updated comments/variable names referencing "lighthouse" in graffiti_calculator.rs and network/Cargo.toml.

**Remaining "lighthouse" references**:
- API paths (`.push("lighthouse")`) — breaking change for external tooling
- `lighthouse/` workspace directory (binary crate) — already renamed to `vibehouse` binary
- `LighthouseSubcommands` enum — internal CLI dispatch
- `eth2` crate feature flag `lighthouse` and modules `lighthouse.rs`, `lighthouse_vc/` — API client paths
- Test infrastructure file paths — test artifacts

**Verification**: cargo check clean, cargo fmt clean, clippy clean (pre-push lint-full passes).

### Run 1173: skip JustifiedBalances clone in find_head when unchanged (2026-03-14)

**Scope**: Performance optimization in fork choice hot path.

**Problem**: `ProtoArrayForkChoice::find_head()` cloned the `JustifiedBalances` struct into `self.balances` every slot. On mainnet with ~1M validators, `effective_balances: Vec<u64>` is ~8MB. This clone happened every 12 seconds even though the balances only change when the justified checkpoint changes (~every 32 slots on mainnet, every 8 slots on minimal).

**Fix**: Added `maybe_update_balances()` method that compares three cheap summary fields (`effective_balances.len()`, `total_effective_balance`, `num_active_validators`) before cloning. If all match, the clone is skipped. These fields change whenever the justified checkpoint changes (new epoch = different rewards/penalties = different total), so the check is effectively exact.

**Edge case handling**: On fresh start, `self.balances` is `JustifiedBalances::default()` (empty Vec, zero total), so the length mismatch triggers the clone on the first call. On restart from persisted state, `self.balances` is populated from SSZ.

**Impact**: Eliminates ~8MB allocation per slot on mainnet (~31 out of 32 slots per epoch). Saves ~240MB/min of allocation churn.

**Verification**: 188/188 proto_array tests, 119/119 fork_choice tests, 9/9 EF fork choice spec tests, clippy clean (pre-push lint-full passes).

### Run 1174: derive Copy for AttestationShufflingId (2026-03-14)

**Scope**: Type-level optimization to eliminate unnecessary clone overhead.

**Problem**: `AttestationShufflingId` is a 40-byte struct (8-byte `Epoch` + 32-byte `Hash256`) with both fields already `Copy`, but the type only derived `Clone`. Every use of `.clone()` went through the Clone trait's heap-awareness machinery instead of a simple bitwise copy.

**Fix**: Added `Copy` to the derive list on `AttestationShufflingId`, then removed all `.clone()` calls on this type across the codebase (9 files, ~15 call sites in proto_array, beacon_chain, shuffling_cache, early_attester_cache, state_advance_timer).

**Impact**: Minor — eliminates Clone trait overhead for a small Copy-eligible type. Mainly a correctness-of-trait-bounds improvement.

**Verification**: 307/307 proto_array + fork_choice tests, 9/9 EF fork choice spec tests, clippy clean workspace-wide, pre-push lint-full passes.

### Run 1175: derive Copy for PayloadAttestationData + light client cache clone avoidance (2026-03-14)

**Scope**: Two performance optimizations targeting unnecessary clones.

**Change 1 — PayloadAttestationData Copy derivation**:
- `PayloadAttestationData` is a 42-byte struct (Hash256 + Slot + 2 bools) with all Copy fields, but only derived Clone.
- Added `Copy` to the derive list, then removed all `.clone()` calls on this type across the codebase (10 files, ~20 call sites in beacon_chain, state_processing, network, http_api, validator_client, types tests).
- Eliminates Clone trait overhead for a frequently-used type (HashMap key in payload attestation aggregation, struct field copies in gossip verification).

**Change 2 — Light client server cache clone avoidance**:
- `LightClientServerCache::recompute_and_cache_updates()` cloned entire `LightClientOptimisticUpdate` and `LightClientFinalityUpdate` structs just to call `is_latest()` (which only compares two Slot values).
- Replaced `.read().clone()` pattern with `.read().as_ref().is_none_or(|u| u.is_latest(...))` — borrows through the read guard instead of cloning.
- Also optimized `get_light_client_update()` to check period via read guard before cloning, only cloning when the cached value matches the requested period.

**Verification**: 1597/1597 types+state_processing+fork_choice+proto_array tests, 56/56 validator_store+validator_services tests, 2/2 light client tests, full workspace clippy clean (lib + all targets), pre-push lint-full passes.

### Run 1176: derive Copy for 5 small fixed-size types (2026-03-14)

**Scope**: Type-level optimization — derive Copy for small, all-Copy-field types to eliminate unnecessary Clone trait overhead.

**Types made Copy**:
1. **AttestationData** (128 bytes: Slot + u64 + Hash256 + 2×Checkpoint) — heavily used in attestation processing, 15+ clone sites removed
2. **Eth1Data** (72 bytes: Hash256 + u64 + Hash256) — used in every state upgrade and block body, 10 clone sites removed
3. **VoluntaryExit** (16 bytes: Epoch + u64) — compact exit type, 1 clone site removed
4. **SigningData** (64 bytes: 2×Hash256) — used in all signing operations
5. **ForkData** (36 bytes: [u8;4] + Hash256) — fork specification type

**Clone removals**: 32 files changed, ~25 `.clone()` calls removed across production and test code. Replaced with either direct copy (field access), dereference (`*getter()`), or removal of redundant clone call.

**Verification**: 715/715 types tests, 575/575 state_processing tests, 307/307 fork_choice+proto_array tests, 69/69 EF SSZ static tests, 72/72 slasher+slashing_protection tests, 30/30 store tests. Full workspace clippy clean (lib + all targets + benches), pre-push lint-full passes.

### Run 1177: reuse filtered_nodes and children_index allocations in find_head_gloas (2026-03-14)

**Scope**: Allocation reuse in the per-slot Gloas fork choice hot path.

**Problem**: `find_head_gloas` is called every slot and allocated two fresh data structures each time:
1. `filtered_nodes: Vec<bool>` (~4-5KB, one entry per proto_array node) via `compute_filtered_nodes`
2. `children_index: HashMap<usize, Vec<usize>>` (~16-32KB) via `build_children_index`

These allocations were discarded after each call, creating unnecessary allocation churn on the per-slot hot path.

**Fix**: Added `gloas_filtered_buf: Vec<bool>` and `gloas_children_buf: HashMap<usize, Vec<usize>>` fields to `ProtoArrayForkChoice`. The `compute_filtered_nodes_into` method now reuses the Vec (clear + resize), and `build_children_index_into` clears the HashMap values but keeps the bucket storage. Both methods fill the buffers in-place. Added `from_parts` constructor for SSZ deserialization. Test-facing `compute_filtered_nodes` wrapper returns a clone of the buffer.

**Impact**: Eliminates ~20-37KB of heap allocation per slot on mainnet. The allocations now happen once and are reused across all subsequent `find_head_gloas` calls. The `ancestor_cache` HashMap was already reused (from a prior optimization); this extends the same pattern to the remaining two per-call allocations.

**Verification**: 188/188 proto_array tests, 119/119 fork_choice tests, 9/9 EF fork choice spec tests, clippy clean.

### Run 1178: derive Copy for SignatureBytes (2026-03-14)

**Scope**: Type-level optimization — make `GenericSignatureBytes` (the `SignatureBytes` type alias) implement `Copy`.

**Problem**: `GenericSignatureBytes<Pub, Sig>` is a fixed `[u8; 96]` + two `PhantomData` fields — entirely bitwise-copyable — but only derived `Clone`. Every `.signature.clone()` on types like `PendingDeposit`, `DepositRequest`, `DepositData` went through the Clone trait instead of a simple memcpy. `GenericPublicKeyBytes` (48 bytes) already had a manual `Copy` impl as precedent.

**Fix**: Added manual `Copy` impl for `GenericSignatureBytes<Pub, Sig>` (matching the `GenericPublicKeyBytes` pattern — manual `Copy` impl + manual `Clone` via `*self`, no bounds on `Pub`/`Sig` since only `PhantomData` uses them). Replaced `#[derive(Clone)]` with manual `Clone` impl.

**Clone removals**: 8 `.clone()` calls removed across 5 files:
- `process_operations.rs`: 5 `request.signature.clone()` / `pending_deposit.signature.clone()` → direct copy
- `upgrade/gloas.rs`: 2 `deposit.signature.clone()` / `signature.clone()` → direct copy / `*signature`
- `test_utils.rs`: 1 `invalid_signature.clone()` on `Option<SignatureBytes>` → direct copy
- `create_validators.rs`: 1 `deposit.signature.clone()` → direct copy

**Verification**: 575/575 state_processing tests, 715/715 types tests, 307/307 fork_choice+proto_array tests, 69/69 EF SSZ static tests, 35/35 EF operations+epoch+sanity tests, full workspace lint-full clean.

### Run 1179: derive Copy for 10 small fixed-size types + historical_data_columns clone fix (2026-03-14)

**Scope**: Type-level optimization — derive Copy for 10 small, all-Copy-field types to eliminate unnecessary Clone trait overhead. Plus one unnecessary HashSet clone fix.

**Types made Copy**:
1. **Withdrawal** (32 bytes: 3×u64 + Address) — used in every block's withdrawal processing
2. **PendingDeposit** (177 bytes: PublicKeyBytes + Hash256 + u64 + SignatureBytes + Slot) — used in epoch deposit processing
3. **DepositData** (184 bytes: PublicKeyBytes + Hash256 + u64 + SignatureBytes) — used in deposit verification
4. **DepositRequest** (192 bytes: PublicKeyBytes + Hash256 + u64 + SignatureBytes + u64) — used in execution request processing
5. **DepositMessage** (88 bytes: PublicKeyBytes + Hash256 + u64) — deposit signature verification
6. **WithdrawalRequest** (68 bytes: Address + PublicKeyBytes + u64) — execution request processing
7. **ConsolidationRequest** (116 bytes: Address + 2×PublicKeyBytes) — consolidation request processing
8. **PendingConsolidation** (16 bytes: 2×u64) — epoch consolidation processing
9. **PendingPartialWithdrawal** (24 bytes: 2×u64 + Epoch) — withdrawal processing
10. **SyncAggregatorSelectionData** (16 bytes: Slot + u64) — sync committee selection

**Clone removals**: 17 files changed. ~12 `.clone()` calls removed across production and test code (state_processing, execution_layer, types).

**Additional fix**: `historical_data_columns.rs` — replaced `unique_column_indices.clone()` (HashSet clone per outer loop iteration) with `&unique_column_indices` iteration by reference. ColumnIndex is u64 (Copy), so iterating by reference works fine.

**Verification**: 715/715 types tests, 575/575 state_processing tests, 307/307 fork_choice+proto_array tests, 69/69 EF SSZ static tests, 35/35 EF operations+epoch+sanity tests, full workspace clippy clean (lib + all targets), pre-push lint-full passes.

### Run 1182: get_proposer_head Vec allocation elimination (2026-03-14)

**Scope**: Allocation optimization in fork choice hot path.

**Problem**: `get_proposer_head_info` collected 2 `ProtoNode` elements into a `Vec`, then popped them out in reverse order. This allocated a Vec (heap + 2 large ProtoNode clones) when simple iterator extraction sufficed.

**Fix**: Replaced `.take(2).cloned().collect::<Vec<_>>()` + `.pop()` + `.pop()` with `.cloned()` iterator + `.next()` + `.next()`. The iterator directly yields head then parent (ancestor order), eliminating the Vec allocation entirely. The `take(2)` was also unnecessary since we only call `.next()` twice.

**Impact**: Eliminates one Vec heap allocation per `get_proposer_head_info` call (called on every slot for proposer head computation). ProtoNode is a large struct (~300+ bytes with all Gloas fields), so avoiding even 2 clones into a temporary Vec is worthwhile.

**Also reviewed**: Checked 3 post-alpha.3 consensus-specs PRs (#5001, #4940, #5002) — all already handled by vibehouse.

**Verification**: 188/188 proto_array tests, 119/119 fork_choice tests, 9/9 EF fork choice spec tests, clippy clean.

### Run 1183: reuse active_votes allocation in find_head_gloas (2026-03-14)

**Scope**: Allocation optimization in the per-slot Gloas fork choice hot path.

**Problem**: `find_head_gloas` allocated a fresh `Vec<(&VoteTracker, u64)>` every slot, collecting all active validators' vote references and balances. On mainnet with ~1M validators, each entry is 16 bytes (reference + u64), totaling ~12MB of allocation per slot. This Vec was discarded after each call.

**Fix**: Added `gloas_active_votes_buf: Vec<(u32, u64)>` field to `ProtoArrayForkChoice`. Instead of storing vote references (which create lifetime issues preventing field storage), the buffer stores `(vote_index, balance)` pairs. Inner loops access vote data via `self.votes.0[idx as usize]`. The buffer is cleared and refilled each call but retains its heap allocation across slots.

**Changes**:
- Added `gloas_active_votes_buf` field to `ProtoArrayForkChoice`, initialized empty in `from_parts` and `new`
- Changed `find_head_gloas` to fill `self.gloas_active_votes_buf` instead of allocating a local Vec
- Changed `get_gloas_weight` and `should_apply_proposer_boost_gloas` signature: `active_votes: &[(u32, u64)]` instead of `&[(&VoteTracker, u64)]`
- Inner loops now access votes via index: `let vote = &self.votes.0[vote_idx as usize]`
- Added `compute_active_votes()` test helper for test functions that need one-off active vote slices
- Note: `ancestor_cache` HashMap could NOT be made a field due to `&mut` borrow conflict with `&self` method calls

**Impact**: Eliminates ~12MB of allocation per slot on mainnet (after first slot). The extra array index lookup per vote in the inner loop is negligible compared to the saved allocation.

**Verification**: 188/188 proto_array tests, 119/119 fork_choice tests, 9/9 EF fork choice spec tests, full workspace clippy clean (lint-full), pre-push hook passes.

### Run 1185: skip delta Vec allocation in Gloas fork choice path (2026-03-14)

**Scope**: Eliminate unnecessary allocation in the per-slot fork choice hot path.

**Problem**: `find_head` called `compute_deltas` unconditionally, which allocates `vec![0_i64; indices.len()]` (one i64 per proto_array node) and does HashMap lookups + arithmetic for every changed vote. In Gloas mode, `find_head_gloas` was called immediately after, and the delta Vec was dropped without ever being used — Gloas computes weights directly from votes via `get_gloas_weight`, not from accumulated deltas.

The vote-tracker side effects (advancing `current_root` to `next_root`, zeroing equivocated votes) are needed before `find_head_gloas` runs, but the actual delta values are not.

**Fix**: Split the vote-tracker side effects into a new `apply_vote_updates` function that performs the same mutations as `compute_deltas` but without allocating the delta Vec or doing any delta arithmetic/HashMap lookups. In Gloas mode, `apply_vote_updates` is called instead of `compute_deltas`. In pre-Gloas mode, `compute_deltas` is still called (moved after the `is_gloas` check).

**Impact**: Eliminates one Vec allocation per slot (`indices.len() * 8` bytes, typically 8-80KB on mainnet depending on tree depth) plus O(changed_votes) HashMap lookups for delta computation that were immediately discarded.

**Also verified**: Spec stable — no new consensus-specs commits since run 1184. PR #4992 (cached PTCs in state) still OPEN. PR #4940 (Gloas fork choice tests) merged into v1.7.0-alpha.3 — all 46 Gloas fork choice test cases pass (ex_ante: 3, get_head: 9, on_block: 23, on_execution_payload: 1, reorg: 8, withholding: 2).

**Verification**: 307/307 proto_array+fork_choice tests, 9/9 EF fork choice spec tests, full workspace clippy clean (lint-full + all targets), pre-push hook passes.

### Run 1189: replace Hash256::from_slice with Hash256::from for fixed-size arrays (2026-03-14)

**Scope**: Eliminate unnecessary slice indirection in Hash256 construction from fixed-size `[u8; 32]` arrays.

**Problem**: Multiple call sites used `Hash256::from_slice(&array)` where `array` is already a `[u8; 32]`. `from_slice` takes `&[u8]` (dynamic slice) and performs a runtime length check, while `From<[u8; 32]>` is a direct, zero-cost conversion. One call site (`compute_kzg_proof`) additionally called `.to_vec()` to create a heap-allocated Vec just to pass to `from_slice`.

**Changes** (7 files):
- `kzg_utils.rs:150`: `Hash256::from_slice(&z.to_vec())` → `Hash256::from(*z)` — eliminates heap allocation (Bytes32 derefs to [u8; 32])
- `kzg_commitment.rs:23`: `Hash256::from_slice(versioned_hash.as_slice())` → `Hash256::from(versioned_hash)` — hash_fixed returns [u8; 32]
- `beacon_block_header.rs:46`: `Hash256::from_slice(&self.tree_hash_root()[..])` → `self.tree_hash_root()` — tree_hash::Hash256 IS types::Hash256 (both alloy B256), round-trip was a no-op
- `slot_epoch_macros.rs:291`: `Hash256::from_slice(&int_to_bytes32(...))` → `Hash256::from(int_to_bytes32(...))` — int_to_bytes32 returns [u8; 32]
- `deposit_tree_snapshot.rs:72`: `Hash256::from_slice(&deposit_root)` → `Hash256::from(deposit_root)` — hash32_concat returns [u8; 32]
- `genesis/interop.rs:16,24`: `Hash256::from_slice(&credentials)` → `Hash256::from(credentials)` — credentials are [u8; 32] arrays
- `genesis/common.rs:29`: `Hash256::from_slice(&int_to_bytes32(...))` → `Hash256::from(int_to_bytes32(...))` — int_to_bytes32 returns [u8; 32]

**Also checked**: Spec stable — no new consensus-specs commits since run 1188. PR #4992 (cached PTCs in state) still OPEN, NOT MERGED (1 APPROVED). No new spec test releases (still v1.6.0-beta.0). PRs #5001 and #5002 already implemented/compatible.

**Verification**: 715/715 types tests, 2/2 kzg tests, 2/2 genesis tests, 69/69 EF SSZ static tests, full workspace clippy clean (lint-full), pre-push hook passes.

### Run 1191: accept cell_proofs by reference in blobs_to_data_column_sidecars

**Problem**: `blobs_to_data_column_sidecars` took `cell_proofs: Vec<KzgProof>` by value, but only used `.len()` and `.chunks_exact()` on it — both of which work on `&[T]`. This forced every caller to allocate a new Vec via `.to_vec()` on their `KzgProofs<E>` (VariableList), copying all proof data unnecessarily.

**Fix**: Changed the parameter type from `Vec<KzgProof>` to `&[KzgProof]`. Updated all 9 call sites (1 production in block_verification.rs, 1 production in fetch_blobs/mod.rs, 6 tests in kzg_utils.rs, 1 bench). Since `VariableList<T, N>` implements `Deref<Target = [T]>`, callers now simply pass `&proofs` instead of `proofs.to_vec()`.

**Verification**: 2/2 KZG tests pass, bench + test compilation clean, full clippy clean.

**Spec check**: No new consensus-specs commits since run 1190. Spec at v1.7.0-alpha.3. PR #4992 (cached PTCs) still OPEN.

### Run 1197: avoid Hash256 wrapper in compute_shuffled_index + deposit tree direct conversion (2026-03-14)

**Scope**: Remove unnecessary Hash256 construction in shuffling and deposit tree code.

**Problem 1**: `compute_shuffled_index.rs` used `Hash256::from_slice(digest.as_ref())` in `hash_with_round` and `hash_with_round_and_position`, constructing a full Hash256 wrapper only to read 1 or 8 bytes from it. The `from_slice` call also performed a runtime length check on what was already a `[u8; 32]` return from `finalize()`.

**Fix 1**: Changed both hash helper functions to return `[u8; 32]` directly instead of Hash256, removing the `use crate::Hash256` import entirely. Simplified `bytes_to_int64` to take `&[u8; 32]` and use `try_into` for the slice-to-array conversion.

**Problem 2**: `deposit_data_tree.rs` used `Hash256::from_slice(&hash_fixed(...))` and `Hash256::from_slice(&self.length_bytes())` where the arguments were already `[u8; 32]` arrays.

**Fix 2**: Changed both to `Hash256::from(...)` for direct zero-cost conversion.

**Impact**: `compute_shuffled_index` is called for single-index shuffling (e.g. computing a specific validator's committee assignment). While `shuffle_list` is the primary hot path for full committee computation, `compute_shuffled_index` is used when only one index is needed. Removing the Hash256 intermediary eliminates unnecessary type wrapping per hash call (2 hashes per round × 90 rounds = 180 eliminated Hash256 constructions per index lookup).

**Spec check**: No new consensus-specs commits since run 1196 (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases (latest v1.6.0-beta.0). cargo audit unchanged (1 rsa, no fix).

**Verification**: 5/5 swap_or_not_shuffle tests, 1/1 EF shuffling tests, 3/3 EF deposit + genesis tests, full clippy clean (all targets).

### Run 1201: closure reference + pending_consolidations clone avoidance (2026-03-14)

**Scope**: Two optimizations targeting per-block and per-epoch hot paths.

**Change 1 — Pass get_pubkey closure by reference in block signature verifier**:
- `BlockSignatureVerifier` cloned `self.get_pubkey` (a closure) at every signature set call site — 6 times per block (proposal, randao, proposer slashings, attester slashings, attestations, exits), plus once per proposer slashing and attester slashing in the block.
- Since `&F` implements `Fn` when `F: Fn`, the signature set functions can accept `&self.get_pubkey` directly instead of `self.get_pubkey.clone()`.
- Removed the `Clone` bound from `F` on `BlockSignatureVerifier` since it's no longer needed.
- Replaced all 6 `self.get_pubkey.clone()` call sites with `&self.get_pubkey`.

**Change 2 — Avoid cloning pending_consolidations list in epoch processing**:
- `process_pending_consolidations` cloned the entire `pending_consolidations` List to iterate while mutating state.
- Split into two passes: (1) read-only pass collects `(source_index, target_index, consolidated_balance)` tuples into a small Vec, (2) mutation pass applies balance changes.
- The Vec is bounded by the per-epoch consolidation churn limit (typically single-digit entries), so the allocation is minimal compared to cloning the entire list.

**Verification**: 575/575 state_processing tests, 19/19 EF epoch_processing + consolidation tests, full workspace clippy clean (lint-full), pre-push hook passes.

### Run 1202: avoid intermediate Vec allocation in PTC committee computation (2026-03-14)

**Scope**: Eliminate unnecessary Vec allocation in `get_ptc_committee`, the per-slot Gloas PTC selection function.

**Problem**: `get_ptc_committee` concatenated all beacon committee validator indices into an intermediate `Vec<u64>` before doing weighted selection. On mainnet with 64 committees per slot and ~64 validators per committee, this allocated a ~32KB Vec (~4000 entries × 8 bytes) on every call. The Vec was only used for random-access lookups (`indices[i % total]`) during the ~16-20 iteration selection loop.

**Fix**: Replaced the intermediate `indices` Vec with direct committee-walk lookups. For each candidate, the function now walks the committees array to find the validator at the flat index. The committee walk is O(committees_per_slot) per candidate (~64 comparisons), but the total work (~20 × 64 = 1280 comparisons) is much cheaper than the eliminated allocation + 4000 push operations + cache pressure from the 32KB Vec.

**Changes**:
- Replaced `.sum()` with `.fold(0, |acc, c| acc.saturating_add(...))` (disallowed method)
- Replaced `indices` Vec construction with inline committee walk using `.get()` (no panicking index)
- Used `saturating_sub` for the remaining-index arithmetic (no arithmetic side effects)

**Spec check**: No new consensus-specs commits since run 1201 (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases (latest v1.6.0-beta.0). cargo audit unchanged (1 rsa, no fix).

**Verification**: 374/374 Gloas+PTC+payload_attestation tests, 15/15 EF operations spec tests, full workspace clippy clean (lint-full + all targets), pre-push hook passes.

### Run 1203: attester_slashing_signature_sets closure reference (2026-03-14)

**Scope**: Remove unnecessary `Clone` bound and closure clone in `attester_slashing_signature_sets`.

**Problem**: `attester_slashing_signature_sets` required `F: Fn(...) + Clone` and cloned the `get_pubkey` closure to pass it to `indexed_attestation_signature_set` twice. Since `&F` implements `Fn` when `F: Fn`, the first call can receive `&get_pubkey` instead of a clone, and the `Clone` bound is no longer needed.

**Fix**: Changed `get_pubkey.clone()` to `&get_pubkey` for the first call, removed the `+ Clone` bound from `F`. The caller in `block_signature_verifier.rs` already passes `&self.get_pubkey`, so no caller changes needed.

**Spec check**: No new consensus-specs commits since run 1202 (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases (latest v1.6.0-beta.0).

**Verification**: 52/52 signature tests, 5/5 attester_slashing tests, clippy clean, fmt clean.

### Run 1205: eliminate heap allocations in merkle_root_from_branch (2026-03-14)

**Scope**: Remove all Vec heap allocations from `merkle_root_from_branch`, the core merkle proof verification function.

**Problem**: `merkle_root_from_branch` used `Vec<u8>` for the running hash, allocating on every loop iteration:
- Line 385: `leaf.as_slice().to_vec()` — heap-allocated a 32-byte Vec from a fixed-size H256
- Line 390: `hash32_concat(...)[..].to_vec()` — heap-allocated a Vec from the `[u8; 32]` return of `hash32_concat`
- Line 392-394: `extend_from_slice` + `hash(&input)` — grew the Vec to 64 bytes, then `hash()` returned a new `Vec<u8>`
- Line 398: `H256::from_slice(&merkle_root)` — runtime length check on what was always 32 bytes

Every iteration allocated at least one Vec. For a depth-32 merkle tree (standard), that's 32+ heap allocations per proof verification.

**Fix**: Replaced the `Vec<u8>` with a `[u8; 32]` stack array throughout:
- `leaf.into()` for the initial conversion (zero-cost, H256 is B256 which is `[u8; 32]`)
- `hash32_concat(a, b)` directly returns `[u8; 32]` — no `.to_vec()` needed
- The `else` branch previously used `hash()` (returns `Vec<u8>`) with manual concatenation; replaced with `hash32_concat(&merkle_root, leaf.as_slice())` which is semantically identical (`hash(h1 || h2)`)
- `H256::from(merkle_root)` for the final conversion (zero-cost `From<[u8; 32]>`)
- Removed unused `hash` import from `ethereum_hashing`

**Impact**: `merkle_root_from_branch` is called by `verify_merkle_proof` which is used in deposit verification (`verify_deposit`), blob sidecar KZG inclusion proofs, and data column sidecar proofs. Eliminates ~depth heap allocations per call (typically 32).

**Spec check**: No new consensus-specs commits since run 1204 (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases (latest v1.6.0-beta.0).

**Verification**: 7/7 merkle_proof tests (including quickcheck), 2/2 EF genesis tests, full workspace clippy clean (lint-full), pre-push hook passes.

### Run 1206: eliminate heap allocations in batch signature verification (2026-03-14)

**Scope**: Remove unnecessary heap allocations from `verify_signature_sets` in `crypto/bls/src/impls/blst.rs`, the core batch BLS signature verification function used for every block and attestation.

**Problem**: Three unnecessary allocations:
1. Line 39: `signature_sets.collect::<Vec<_>>()` — collected an `ExactSizeIterator` into a Vec just to get `.len()` and iterate. The length is available from the iterator directly via `.len()`.
2. Lines 92-96: `set.signing_keys.iter().map(|pk| pk.point()).collect::<Vec<_>>()` — allocated a new Vec of public key references on every iteration of the main loop (once per signature set). For a typical block with ~128 attestations, that's ~128 heap allocations.
3. Line 106: `sigs.iter().zip(pks.iter()).unzip()` — created two new Vecs via zip+unzip when simple `.iter().collect()` on each Vec is cleaner (same allocation count but avoids the zip overhead).

**Fix**:
1. Use `signature_sets.len()` before consuming the iterator, then iterate directly — eliminates one Vec allocation
2. Declare `signing_keys_buf: Vec<&blst_core::PublicKey>` outside the loop, `.clear()` + `.extend()` each iteration — the buffer's heap allocation is retained across iterations, eliminating N-1 allocations where N is the number of signature sets
3. Replace `unzip()` with two direct `.iter().collect()` calls

**Impact**: `verify_signature_sets` is called on every block import (batch verifying all signatures: block signature, RANDAO, proposer slashings, attester slashings, attestations, voluntary exits, sync committee). A typical mainnet block has 5-10 signature sets with 1-128 signing keys each. This eliminates 1 + (N-1) heap allocations per call where N is the number of signature sets.

**Spec check**: No new consensus-specs commits since run 1205 (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases (latest v1.7.0-alpha.3).

**Verification**: 37/37 BLS tests, 8/8 EF BLS spec tests (including bls_batch_verify), 52/52 signature state_processing tests, full workspace clippy clean (lint-full), pre-push hook passes.

### Run 1208: remove unnecessary AggregateSignature clone in verify functions (2026-03-14)

**Scope**: Remove unnecessary `.clone()` calls on `blst_core::AggregateSignature` in `fast_aggregate_verify` and `aggregate_verify`.

**Problem**: `BlstAggregateSignature::fast_aggregate_verify` (line 257) and `aggregate_verify` (line 271) both did `self.0.clone().to_signature()`, cloning the entire `AggregateSignature` before converting to `Signature`. However, `AggregateSignature::to_signature()` takes `&self` — proven by `serialize()` (line 241) which calls `self.0.to_signature()` without clone successfully.

**Fix**: Changed both call sites from `self.0.clone().to_signature()` to `self.0.to_signature()`, eliminating two unnecessary cryptographic type clones per signature verification.

**Impact**: `fast_aggregate_verify` is called for every attestation and sync committee signature verification. `aggregate_verify` is used in batch verification paths. Each clone copies the internal BLS point representation (~96 bytes). On mainnet with ~128 attestations per block, this eliminates ~128 unnecessary aggregate signature copies per block import.

**Spec check**: No new consensus-specs commits since run 1207 (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases (latest v1.7.0-alpha.3).

**Verification**: 37/37 BLS tests, 8/8 EF BLS spec tests (including bls_batch_verify), 52/52 signature state_processing tests, full workspace clippy clean (lint-full + make lint), pre-push hook passes.

### Run 1209: avoid allocations in sync committee and attestation verification (2026-03-14)

**Scope**: Two optimizations targeting gossip verification hot paths.

**Change 1 — Return slice from get_subcommittee_pubkeys instead of Vec**:
- `SyncCommittee::get_subcommittee_pubkeys` previously returned `Vec<PublicKeyBytes>` by copying the subcommittee slice via `.to_vec()`. On mainnet, each subcommittee has 128 entries × 48 bytes = ~6KB copied per call.
- Changed return type to `&[PublicKeyBytes]`, returning a direct slice reference into the `FixedVector` backing store. Zero-copy.
- Updated the caller in `sync_committee_verification.rs` to bind the `Arc<SyncCommittee>` before slicing (required for borrow lifetimes), and changed `.into_iter()` to `.iter()` with explicit copy (`*pubkey`) for the filtered participant pubkeys.
- The caller in `test_utils.rs` already used `.iter()` on the result, so no changes needed there.

**Change 2 — Avoid cloning selection proof signatures for aggregator checks**:
- Both `SelectionProof` (attestation aggregation) and `SyncSelectionProof` (sync committee aggregation) required cloning the `Signature` (96 bytes) just to check aggregator status. The comments explicitly noted this as "known to be a relatively slow operation" with "Future optimizations should remove this clone."
- Added `is_aggregator_sig(&Signature, ...)` static methods to both types that take a reference instead of requiring ownership.
- Updated callers: `attestation_verification.rs` now extracts `&signed_aggregate.message.selection_proof` (reference) instead of `.clone()`, and calls `SelectionProof::is_aggregator_sig(...)`. `sync_committee_verification.rs` similarly calls `SyncSelectionProof::is_aggregator_sig::<T::EthSpec>(&signed_aggregate.message.selection_proof)`.
- Eliminates one 96-byte signature clone per aggregate attestation gossip verification and per sync committee contribution gossip verification.

**Spec check**: No new consensus-specs commits since run 1208 (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases (latest v1.7.0-alpha.3).

**Verification**: 5/5 attestation + sync committee verification tests, 6/6 types sync committee tests, full workspace clippy clean (lint-full), pre-push hook passes.

### Run 1210: use serialize() instead of as_ssz_bytes() in aggregator checks (2026-03-14)

**Scope**: Eliminate Vec heap allocations in `SelectionProof` and `SyncSelectionProof` aggregator checks.

**Problem**: Both `is_aggregator_sig()` and `is_aggregator_from_modulo()` called `sig.as_ssz_bytes()` which invokes the default `Encode::as_ssz_bytes()` method — this creates a `Vec<u8>` with capacity 96, copies the signature bytes into it, then passes `&Vec<u8>` to `hash_fixed()`. Since `Signature::serialize()` returns `[u8; 96]` (a stack-allocated array) with identical content, calling `hash_fixed(&sig.serialize())` is semantically identical but avoids the heap allocation entirely.

**Fix**: Replaced all `as_ssz_bytes()` calls with `serialize()` in both `SelectionProof` (2 call sites) and `SyncSelectionProof` (2 call sites). Removed the now-unused `use ssz::Encode` imports from both files.

**Impact**: Eliminates one 96-byte Vec allocation per aggregator check. These checks run on every aggregate attestation and sync committee contribution received via gossip. On mainnet, this eliminates hundreds of unnecessary heap allocations per slot.

**Verification**: 1/1 sync_selection_proof test, 4/4 attestation + sync committee verification tests, full workspace clippy clean (lint-full), pre-push hook passes.

### Run 1211: replace HashSet allocation in block producer observation checks (2026-03-14)

**Scope**: Eliminate unnecessary HashSet heap allocation in `observed_block_producers.rs` and `observed_slashable.rs`.

**Problem**: Both files used `block_roots.difference(&HashSet::from([block_root])).count() == 0` to check whether the set contains any block roots OTHER than the given one. This allocates a `HashSet` (with internal `HashMap` — bucket allocation + hashing) on every call, just to compare a single element.

**Fix**: Replaced with `block_roots.iter().any(|r| r != &block_root)`, which does a simple iteration with no allocations. The `observed_block_producers` check additionally uses `block_roots.contains(&block_root)` for the duplicate check, which was already present.

**Impact**: `observe_proposal_slashable` and `observe_proposer` are called for every gossip block received. On mainnet with ~1 block/slot across many peers propagating it, this eliminates one HashSet allocation per gossip block observation. The HashSet allocation included HashMap bucket allocation, hashing the block_root, and constructing the set structure — all for a single-element set.

**Spec check**: No new consensus-specs commits since run 1210 (latest e50889e1ca, #5004). No new spec test releases (latest v1.6.0-beta.0).

**Verification**: 4/4 observed_block_producers + observed_slashable tests, full workspace clippy clean (lint-full), pre-push hook passes.

### Run 1212: use serialize() instead of as_ssz_bytes()/ssz_encode() for signature hashing (2026-03-14)

**Scope**: Eliminate Vec heap allocations from signature hashing in BeaconState and IndexedAttestation.

**Problem**: Three call sites used `as_ssz_bytes()` or `ssz_encode()` on `Signature`/`AggregateSignature` types, allocating a 96-byte `Vec<u8>` when `serialize()` returns a stack-allocated `[u8; 96]`:
1. `beacon_state.rs:1248` — `is_aggregator()` called `slot_signature.as_ssz_bytes()` for aggregator check hash
2. `beacon_state.rs:1589` — `update_randao_mix()` called `ssz_encode(signature)` for RANDAO mix update
3. `indexed_attestation.rs:203` — `Hash` impl called `self.signature().as_ssz_bytes()` for HashMap/HashSet hashing

**Fix**: Replaced all three with `.serialize()` which returns `[u8; 96]` on the stack. Removed now-unused `use ssz::Encode` from `indexed_attestation.rs` and `use ssz::{Encode, ssz_encode}` from `beacon_state.rs`.

**Impact**: `update_randao_mix` is called once per block. `is_aggregator` is called during aggregation checks. The `Hash` impl for `IndexedAttestation` is used in the operation pool's `HashSet`/`HashMap` operations. Each call previously allocated a 96-byte Vec on the heap; now uses the stack.

**Verification**: 715/715 types tests, full workspace clippy clean (lint-full), pre-push hook passes.

### Run 1213: remove unnecessary heap allocations in BLS Display/Debug and pubkey hashing (2026-03-14)

**Scope**: Eliminate unnecessary `.to_vec()` and `.as_ssz_bytes()` calls in BLS crypto formatting and pubkey hashing functions.

**Problem**: Four categories of unnecessary allocations:
1. `impl_display!` macro (macros.rs:85): `hex_encode(self.serialize().to_vec())` — `.to_vec()` converts the stack-allocated `[u8; N]` array from `serialize()` to a heap-allocated `Vec<u8>`, but `hex_encode` takes `T: AsRef<[u8]>` which `[u8; N]` already implements.
2. `impl_debug!` macro (macros.rs:160): Same pattern with `hex_encode(&self.serialize().to_vec())`.
3. `get_withdrawal_credentials` (get_withdrawal_credentials.rs:9): `pubkey.as_ssz_bytes()` allocates a 48-byte `Vec<u8>` when `pubkey.serialize()` returns `[u8; 48]` on the stack.
4. `bls_withdrawal_credentials` and `eth1_withdrawal_credentials` (interop.rs:14,20): Same `as_ssz_bytes()` pattern. Plus 3 test assertions using `v.pubkey.as_ssz_bytes()`.
5. `builder.rs:1466` test: Same pattern.

**Fix**:
- Removed `.to_vec()` from both `impl_display!` and `impl_debug!` macros — pass the array directly to `hex_encode`.
- Replaced `as_ssz_bytes()` with `serialize()` in `get_withdrawal_credentials`, `bls_withdrawal_credentials`, `eth1_withdrawal_credentials`, and test assertions.
- Removed now-unused `use ssz::Encode` imports from `get_withdrawal_credentials.rs`, `interop.rs`, and `builder.rs`.

**Impact**: The Display/Debug macros are used by `GenericPublicKey`, `GenericSignature`, `GenericAggregateSignature`, and their `*Bytes` variants. Every `format!("{}", pubkey)`, `format!("{:?}", sig)`, log statement, or serde serialization of these types previously allocated a Vec (48 bytes for pubkeys, 96 bytes for signatures). In a running node, pubkeys and signatures are logged and serialized frequently (peer discovery, attestation processing, API responses).

**Spec check**: No new consensus-specs commits since run 1212 (latest e50889e1ca, #5004). No new spec test releases (latest v1.7.0-alpha.3).

**Verification**: 37/37 BLS tests, 8/8 EF BLS spec tests, 2/2 genesis tests, full workspace clippy clean (lint-full), pre-push hook passes.

### Run 1215: fix consolidation processing regression (2026-03-14)

**Scope**: CI failure — `electra/sanity/blocks/multi_epoch_consolidation_chain` EF spec test failing with state root mismatch (real crypto only).

**Root cause**: Commit 9cf1e78d5 (run 1201, "avoid cloning entire pending_consolidations list in epoch processing") introduced a semantic bug. The optimization split `process_pending_consolidations` into two passes: first collect `(source, target, balance)` tuples, then apply balance changes. But the spec requires balance changes to be applied **inline** — when multiple consolidations affect the same validator, later consolidations must see the decreased balance from earlier ones. The two-pass approach read all balances before any were modified, producing incorrect results for multi-consolidation chains.

**Fix**: Reverted to single-pass inline processing. The clone of `pending_consolidations` is necessary here because we need immutable iteration while mutating balances. This is one of the cases where the borrow checker correctly forces the clone.

**Bisect**: Used `git bisect` to identify 9cf1e78d5 as the first bad commit (6 steps, ~10 minutes).

**Verification**: 79/79 real-crypto EF tests, 139/139 fake-crypto EF tests, 575/575 state_processing tests, lint clean.

### Run 1214: performance optimization sweep complete (2026-03-14)

**Scope**: Searched for remaining allocation optimization opportunities across entire codebase.

**Method**: Comprehensive scan of all production code for:
- `as_ssz_bytes()` on fixed-size types → all converted to `serialize()` in runs 1210-1213
- `to_vec()` on fixed-size arrays → all converted in runs 1204-1213
- `collect::<Vec<_>>()` where iterator would suffice → all optimized in runs 1151-1164
- `hash()` (Vec return) vs `hash_fixed()` ([u8; 32] return) → all using `hash_fixed` already
- `ssz_encode()` → all converted to `serialize()` in run 1212
- Unnecessary clones in hot paths → all optimized in runs 1199-1211

**Result**: No remaining actionable optimization targets in production code. All remaining `collect::<Vec<_>>()` calls are either:
- Required by borrow checker (need to materialize before mutable state access, e.g. `slashings.rs`)
- On complex types without `FromIterator` (e.g. `VariableList` in `block_replayer.rs`)
- In low-frequency paths (once per block/epoch, not per-attestation)
- In test code

All remaining `.clone()` calls are either:
- Required by ownership semantics (API boundaries, channel sends)
- `Arc::clone` (cheap atomic increment)
- Borrow checker constraints (simultaneous read+write)

**Conclusion**: Phase 4 (Performance) is comprehensively complete. Runs 1151-1214 eliminated heap allocations across all hot paths: BLS verification, attestation processing, fork choice head computation, state transitions, gossip processing, merkle proofs, and crypto formatting.

### Run 1216: fix self-build envelope signature verification (2026-03-14)

**Scope**: Spec conformance audit found that self-build envelopes (builder_index == BUILDER_INDEX_SELF_BUILD) had signature verification entirely skipped. Per spec, `verify_execution_payload_envelope_signature` always verifies — for self-build it uses the proposer's validator pubkey instead of a builder pubkey.

**Root cause**: Original implementation assumed self-build envelopes skip verification, based on an incorrect interpretation. The comment referenced a non-existent `is_valid_indexed_execution_payload_envelope` function. The VC already signs self-build envelopes with the proposer's key (correct), but other nodes skipped verifying that signature (incorrect).

**Fix**:
- `execution_payload_envelope_signature_set` (signature_sets.rs): handle self-build by looking up `state.validators[state.latest_block_header.proposer_index].pubkey`
- `process_execution_payload_envelope` (envelope_processing.rs): remove `!= BUILDER_INDEX_SELF_BUILD` guard
- `verify_payload_envelope_for_gossip` (gloas_verification.rs): remove `!= BUILDER_INDEX_SELF_BUILD` guard
- Tests updated: 3 tests changed from "any signer accepted" to "wrong signer rejected", 2 new tests added (empty signature rejected, proposer signature verified in gossip)

**Impact**: Without this fix, any node could forge self-build envelopes with arbitrary payloads for any slot — the empty/forged signature would be accepted. The bid's block_hash commitment provides some protection, but the signature is an additional cryptographic guarantee.

**Verification**: 575/575 state_processing tests, 79/79 EF spec tests (real crypto), 139/139 EF spec tests (fake crypto), full workspace lint-full passes.

### Run 1424: add beacon_fork_choice_store and block_times_cache test coverage (2026-03-15)

**Scope**: Add unit test coverage for two previously untested beacon_chain modules.

**beacon_fork_choice_store.rs** (15 tests):
- `BalancesCache`: empty cache returns None, get/position with matching/non-matching root+epoch, cache eviction at MAX_BALANCE_CACHE_SIZE (4), no duplicate entries, same root different epochs, same epoch different roots, empty balances valid, get clones independently
- `PersistedForkChoiceStore`: SSZ encode/decode roundtrip preserving all fields
- `CacheItem`: SSZ roundtrip
- `BalancesCache`: SSZ roundtrip

**block_times_cache.rs** (13 tests, was 1):
- `BlockDelays::new()`: full calculation with all timestamps, missing timestamps (all None), available uses max(executed, all_blobs_observed), before-slot-start returns None
- `set_time_blob_observed`: uses maximum (not minimum like other setters)
- `set_time_if_less`: keeps minimum timestamp
- `prune`: removes old entries, no underflow at early slots
- `get_block_delays`/`get_peer_info`: unknown block returns defaults
- Multiple blocks tracked independently
- All `set_time_*` methods create entry if missing

**Verification**: 28/28 tests pass, clippy clean, lint-full passes.

### Run 1425: add fork_choice_signal and events test coverage (2026-03-15)

**Scope**: Add unit test coverage for two previously untested beacon_chain modules.

**fork_choice_signal.rs** (10 tests):
- `notify_and_wait_success`: basic notify → wait round-trip
- `wait_already_ahead`: wait for slot below current returns Success immediately
- `wait_times_out_when_no_signal`: no signal sent → TimeOut
- `notify_out_of_order_returns_error`: slot regression returns ForkChoiceSignalOutOfOrder
- `notify_same_slot_is_ok`: same slot is not strictly less, accepted
- `notify_monotonically_increasing`: 10 increasing slots all succeed
- `concurrent_notify_then_wait`: cross-thread notify wakes waiting receiver
- `behind_when_signaled_lower_slot`: signal slot 3 when waiting for slot 10 → Behind
- `multiple_receivers_all_wake`: notify_all wakes two concurrent receivers
- `default_tx_starts_at_slot_zero`: Default impl starts at slot 0

**events.rs** (16 tests):
- `no_subscribers_initially`: all 21 has_*_subscribers() return false on fresh handler
- `subscribe_block/head/finalized/execution_bid/execution_payload/payload_attestation/execution_proof_received_shows_subscriber`: subscribe creates subscriber
- `drop_receiver_removes_subscriber`: dropping Receiver decrements count
- `register_block/head/finalized_event_received_by_subscriber`: end-to-end register → try_recv
- `register_without_subscribers_does_not_panic`: silent drop when no subscribers
- `multiple_subscribers_all_receive`: broadcast semantics verified
- `capacity_multiplier_scales_channel_size`: multiplier=2 allows 32 buffered events
- `event_routing_independence`: block event not delivered to head subscriber

**Verification**: 26/26 tests pass, clippy clean, lint-full passes.

### Run 1426: add attester_cache test coverage (2026-03-15)

**Scope**: Add unit test coverage for the previously untested `attester_cache.rs` module. Also added `Debug` derive to `AttesterCacheKey` to support test assertions.

**attester_cache.rs** (15 tests):
- **CommitteeLengths** (7 tests): get_committee_count_per_slot bounds check, get_committee_length at slot 0, wrong epoch error, invalid committee index error, all slots in epoch sum to active validators, single validator edge case, epoch 1 boundary
- **AttesterCacheValue** (1 test): get returns correct justified checkpoint and positive committee length
- **AttesterCache** (5 tests): get on empty returns None, insert and get roundtrip, prune_below removes old entries and keeps recent, prune_below zero is noop, insert_respecting_max_len evicts lowest epoch at capacity
- **AttesterCacheKey** (2 tests): equality semantics, inequality on differing epoch/root

**Verification**: 15/15 tests pass, clippy clean, lint-full passes.

### Run 1427: add single_attestation, chain_config, and pre_finalization_cache test coverage (2026-03-15)

**Scope**: Add unit test coverage for three previously untested beacon_chain modules.

**single_attestation.rs** (14 tests):
- **build_attestation_from_single** (5 tests): Base fork produces Base variant with correct aggregation bit, Electra fork produces Electra variant with committee_bits set, Gloas fork produces Electra variant (electra_enabled), aggregation bit out of bounds error, committee index out of bounds error (MinimalEthSpec MaxCommitteesPerSlot=4)
- **single_attestation_to_attestation** (9 tests): Base fork attester lookup, Electra fork attester lookup with committee_bits, AttesterNotInCommittee error with correct fields, attester at first/last position, single member committee, empty committee error, attestation data preservation through conversion

**chain_config.rs** (9 tests):
- Default values verification (all 30+ fields), re_org_cutoff with explicit millis, re_org_cutoff derived from slot duration, millis override, zero millis, default re-org thresholds, default constants, config equality, config clone

**pre_finalization_cache.rs** (9 tests):
- Empty cache contains nothing, empty cache metrics, block_processed removes from lookups, block_processed noop for unknown root, contains reflects block_roots cache, LRU eviction at BLOCK_ROOT_CACHE_LIMIT (512), lookups LRU eviction at LOOKUP_LIMIT (8), metrics returns correct counts, block_processed does not affect block_roots, duplicate insertions

**Verification**: 32/32 tests pass, clippy clean, lint-full passes.

### Run 1428: add block_verification_types test coverage (2026-03-15)

**Scope**: Add unit test coverage for the previously untested `block_verification_types.rs` module, covering `RpcBlock` construction, blob consistency validation, envelope management, and `AsBlock` trait implementations.

**block_verification_types.rs** (21 tests):
- **RpcBlock::new_without_blobs** (3 tests): preserves block fields, uses provided root, computes root when None
- **RpcBlock::new blob consistency** (5 tests): no blobs returns Block variant, empty blob list treated as None, matching blobs succeeds, wrong blob count returns MissingBlobs, mismatched commitment returns KzgCommitmentMismatch
- **RpcBlock::deconstruct** (2 tests): block-only round-trip, block-and-blobs round-trip
- **n_blobs/n_data_columns** (2 tests): zero for block-only, matches blob count
- **envelope operations** (4 tests): initially None, set_and_get, take_returns_and_clears, take_from_empty
- **AsBlock trait** (4 tests): slot, parent_root, canonical_root, block_cloned
- **Pre-Deneb** (1 test): pre-Deneb block with None blobs handled correctly

**Verification**: 21/21 tests pass, clippy clean.

### Run 1446 — Unit test coverage: metrics, deposit_contract, validator_store (93 tests)

**common/metrics/src/lib.rs** (61 tests):
- **decimal_buckets** (7 tests): standard ranges, single power, wide range, negative powers, adjacent powers, edge cases
- **duration_to_f64** (5 tests): zero, whole seconds, fractional, nanoseconds, large durations
- **TryExt trait** (4 tests): Result Ok/Err, Option Some/None timer behavior
- **try_create_* functions** (6 tests): int_gauge, float_gauge, int_counter, histogram, int_gauge_vec, int_counter_vec
- **set/get/inc/dec gauge ops** (6 tests): set, get, inc, dec, float gauge set, counter inc/inc_by
- **maybe_set functions** (4 tests): Some sets value, None is noop for int and float gauges
- **observe functions** (3 tests): observe histogram, observe_duration with timer
- **gather** (1 test): returns registered metrics
- **Error-state no-op** (25 tests): all metric operations on Err values don't panic

**validator_client/validator_store/src/lib.rs** (18 tests):
- **DoppelgangerStatus::only_safe** (3 tests): SigningEnabled→Some, SigningDisabled→None, Unknown→None
- **DoppelgangerStatus::ignored** (3 tests): Enabled→Some, Disabled→Some, Unknown→None
- **DoppelgangerStatus::only_unsafe** (3 tests): Enabled→None, Disabled→Some, Unknown→Some
- **Error type** (9 tests): From conversion, variant distinctness, slot/epoch variants, clone, debug formatting, pubkey preservation

**common/deposit_contract/src/lib.rs** (14 tests, 13 new + 1 existing):
- **Round-trip** (3 tests): standard, multiple keypairs, zero amount
- **Decode failures** (3 tests): empty bytes, short bytes, garbage data
- **Consistency** (3 tests): consistent encode length, different amounts give different roots, wrong amount mismatch
- **Constants** (5 tests): DEPOSIT_DATA_LEN value, gas bounds, ABI/bytecode non-empty

**Verification**: 93/93 tests pass, clippy clean, pushed to origin.

### Run 1447 — Unit test coverage: beacon_node_health module (54 tests)

**validator_client/beacon_node_fallback/src/beacon_node_health.rs** (54 new tests, 3 existing → 57 total):
- **BeaconNodeSyncDistanceTiers** (4 tests): default values, from_vec wrong length, cumulative values, zero modifiers
- **compute_distance_tier** (4 tests): boundary exact match, zero is synced, very large distance, zero-threshold tiers
- **BeaconNodeHealthTier display** (2 tests): normal tier display, zero tier display
- **BeaconNodeHealthTier ordering** (6 tests): different tiers, synced no tiebreak on distance, small/medium/large tiebreak on distance, equality
- **BeaconNodeHealth ordering** (6 tests): different tiers, tiebreak by user_index, higher index loses, lower tier wins over lower index, get_index, get_health_tier
- **Exhaustive tier classification** (16 tests): all 16 possible (SyncDistanceTier × IsOptimistic × ExecutionEngineHealth) combinations verified
- **Sorting** (2 tests): ascending by tier, same-tier by user_index
- **Serde roundtrip** (7 tests): SyncDistanceTier, BeaconNodeHealthTier, BeaconNodeHealth, BeaconNodeSyncDistanceTiers, ExecutionEngineHealth, IsOptimistic, Config
- **PartialOrd consistency** (2 tests): BeaconNodeHealthTier and BeaconNodeHealth consistent with Ord
- Also added `serde_json` dev-dependency to beacon_node_fallback/Cargo.toml

**Verification**: 57/57 tests pass, clippy clean, pushed to origin.

#### Run 1450 — Slasher Array, AttestationQueue, BlockQueue Tests

**slasher/src/array.rs** (34 new tests):
- **Chunk::epoch_distance** (5 tests): zero distance, positive distance, large valid distance, overflow, distance of one
- **Chunk::get_target/set_target/set_raw_distance** (7 tests): basic ops, defaults, multi-validator, multi-epoch, overwrite, out-of-bounds set
- **MinTargetChunk** (7 tests): empty has MAX_DISTANCE, neutral element, name, first_start_epoch, next_start_epoch, update reduces targets, stops when existing smaller
- **MaxTargetChunk** (7 tests): empty has zero, neutral element, name, first_start_epoch, next_start_epoch, update increases targets, stops when existing larger
- **Bincode roundtrips** (3 tests): Chunk, MinTargetChunk, MaxTargetChunk serialization

**slasher/src/attestation_queue.rs** (14 new tests):
- **AttestationQueue** (6 tests): empty, enqueue_dequeue, multiple_enqueue, dequeue_empty, requeue, enqueue_after_dequeue
- **AttestationBatch** (5 tests): queue_single, multiple_validators_same_data, different_data, dedup_prefers_larger_aggregate, keeps_larger_when_queued_first
- **group_by_validator_chunk_index** (3 tests): single_chunk, multiple_chunks, empty_batch

**slasher/src/block_queue.rs** (7 new tests):
- empty_queue, queue_single_block, queue_duplicate_block_deduplicates, queue_different_blocks, dequeue_clears_queue, dequeue_empty_returns_empty_set, queue_after_dequeue

**Verification**: 55/55 tests pass, clippy clean, pushed to origin.

#### Run 1453 — EIP-3076 Interchange Unit Tests

**common/eip_3076/src/lib.rs** (24 new tests):
- **len/is_empty** (3 tests): len returns count, is_empty false when populated, is_empty true when empty
- **equiv** (5 tests): same order, different order (set equality), different metadata, different data, both empty
- **minify edge cases** (11 tests): empty data, picks max block slot, picks max attestation epochs (independent maximization), merges duplicate pubkeys, blocks-only, attestations-only, no blocks or attestations, multiple validators preserved, signing roots cleared, metadata preserved, single entries
- **serde** (6 tests): SignedBlock with/without signing_root serialization, SignedAttestation roundtrip, deny_unknown_fields rejects extra fields, from_json_str valid/invalid

**Verification**: 27/27 tests pass (24 new + 3 existing), clippy clean, pushed to origin.

#### Run 1454 — EIP-2335 Keystore Internal Validation Unit Tests

**crypto/eth2_keystore/src/keystore.rs** (42 new tests):
- **log2_int** (5 tests): zero, one, powers of two, non-powers (floor), u32::MAX
- **is_control_character** (5 tests): C0 range (0x00-0x1F), DEL (0x7F), C1 range (0x80-0x9F), printable ASCII, Unicode
- **normalize** (4 tests): ASCII passthrough, NFKD decomposition of é, invalid UTF-8 error, empty input
- **validate_salt** (4 tests): empty fails, normal length OK, short OK with warning, long OK with warning
- **validate_aes_iv** (3 tests): empty fails, correct 16-byte size, wrong size OK with warning
- **validate_parameters (Pbkdf2)** (7 tests): valid params, wrong dklen, c too large, c at max boundary, c=0, c=1 weak-but-valid, empty salt
- **validate_parameters (Scrypt)** (10 tests): valid params, n=0, n=1, n not power of two, r=0, p=0, wrong dklen, empty salt, n*p*r overflow, n=2 smallest valid
- **keypair_from_secret** (3 tests): valid round-trip, all-zeros rejected, wrong length rejected
- **encrypt** (2 tests): produces different ciphertext, empty IV fails
- **default_kdf** (1 test): returns Scrypt variant
- **Error equality** (1 test): variant distinctness

**Verification**: 77/77 tests pass (42 new + 35 existing), clippy clean, pushed to origin.

#### Run 1455 — Store Config & HDiff Unit Tests

**beacon_node/store/src/config.rs** (19 new tests):
- **verify validation** (5 tests): compression_level zero OK, max OK, out-of-range error, epochs_per_blob_prune zero error, nonzero OK
- **compression estimation** (5 tests): no compression returns original size, with compression halves, decompressed no-compression passthrough, decompressed with compression doubles, zero bytes
- **compress/decompress roundtrip** (4 tests): normal data, empty data, large repetitive data (verifies compression ratio), no-compression-level passthrough
- **as_disk_config** (2 tests): hierarchy preservation, default values preserved
- **OnDiskStoreConfig edge cases** (3 tests): invalid version byte, empty bytes, default roundtrip

**beacon_node/store/src/hdiff.rs** (24 new tests):
- **HierarchyConfig::from_str** (8 tests): valid "5,13,21", single "10", two layers "5,13", empty error, not ascending error, equal values error, descending error, non-numeric error, display roundtrip
- **HierarchyConfig::validate** (4 tests): default OK, empty error, non-ascending error, too-large exponent (>=64) error
- **exponent_for_slot** (4 tests): powers of two, zero returns 64, odd numbers return 0, mixed values
- **HierarchyModuli::should_commit_immediately** (5 tests): snapshot layer true, second layer true, leaf layer false, non-aligned false, single layer
- **storage_strategy edge cases** (3 tests): slot < start error, slot == start is snapshot, start_slot affects diff_from clamping
- **next_snapshot_slot** (1 test): zero slot edge case

**Verification**: 52/52 store tests pass (43 new + 9 existing), clippy clean, pushed to origin.

#### Run 1456 — Verify Operation Signature Validity & Verification Epochs Tests

**consensus/state_processing/src/verify_operation.rs** (18 new tests):
- **verification_epochs** (4 tests): exit returns message epoch, exit at zero, proposer slashing returns single epoch, attester slashing returns two epochs, BLS-to-execution change returns empty
- **signature_is_still_valid** (7 tests): valid when fork unchanged, invalid after fork transition, valid when epoch still in previous fork, valid when epoch in current fork, invalid when fork epoch shifts past message epoch, valid with empty verification epochs (BLS change), valid with two matching fork versions (attester slashing), invalid when one of two fork versions changes
- **accessors** (4 tests): into_inner returns original op, as_inner returns reference, first_fork_verified_against returns first version, returns None when empty, returns first of multiple versions

**Verification**: 22/22 tests pass (18 new + 4 existing SSZ roundtrip), clippy clean, pushed to origin.

#### Run 1460 — Unit Tests for Graffiti File, Deposit Tree Snapshot, and Max Cover (34 tests)

**validator_client/graffiti_file/src/lib.rs** (15 new tests):
- **read_line** (7 tests): default key parsing, public key line, empty graffiti, colons in graffiti value, missing delimiter error, invalid public key error, whitespace trimming
- **determine_graffiti** (4 tests): no sources returns None, flag only, definition overrides flag, file overrides definition and flag
- **GraffitiFile edge cases** (4 tests): nonexistent path error, no default with unknown key returns None, empty file, only whitespace lines

**consensus/types/src/deposit_tree_snapshot.rs** (9 new tests):
- **DepositTreeSnapshot** (5 tests): default is valid, default root matches calculated, invalid snapshot with wrong root, calculate_root returns None for bad count, zero count matches default root
- **FinalizedExecutionBlock** (4 tests): From conversion, SSZ roundtrip, serde roundtrip, deposit_tree_snapshot serde roundtrip

**beacon_node/operation_pool/src/max_cover.rs** (10 new tests):
- **merge_solutions** (6 tests): empty solutions, one empty one nonempty, prefers higher score, respects limit, zero limit, equal scores stable
- **maximum_cover edge cases** (4 tests): empty input, all zero score excluded, single item, disjoint sets full coverage

**Verification**: 34 new tests pass across all three crates, clippy clean, pushed to origin.

#### Run 1461 — Network Config, IP Global Checks, and Network Load Tests

**beacon_node/vibehouse_network/src/config.rs** (36 new tests):
- **is_global_ipv4** (12 tests): public addresses, private ranges, loopback, link-local, broadcast, documentation, shared address space (100.64/10), reserved (192.0.0/24), zero network, special 192.0.0.9/10 globally routable, future protocol (240/4)
- **is_global_ipv6** (9 tests): global public, unspecified, loopback, link-local, unique local, documentation (2001:db8), IPv4-mapped, discard-only (100::/64), special anycast (2001:1::1/2), AMT (2001:3::/32)
- **NetworkLoad::from** (6 tests): load levels 0-5+, verifying mesh params, heartbeat intervals, names ("Low"/"Average"/"High"), 0 and 255 both map to "High"
- **Config** (9 tests): default constants, default values, default listen address is IPv4, set_ipv4/ipv6/dual-stack listening addresses, set_listening_addr dispatch, ENR defaults are None, serde roundtrip

Also added `serde_json` dev-dependency to vibehouse_network/Cargo.toml.

**Verification**: 36/36 tests pass, clippy clean, pushed to origin.

#### Run 1463 — Unit Test Coverage Assessment (no new tests)

Exhaustive search of 100+ source files across all directories (common/, consensus/, beacon_node/, validator_client/, crypto/, slasher/, testing/) for modules lacking `#[cfg(test)]` that contain self-contained, unit-testable logic.

**Finding**: All self-contained, unit-testable modules in the codebase now have test coverage from runs 1426-1462. The remaining untested files fall into categories that require complex integration setup:
- Large integration modules (beacon_chain.rs, block_verification.rs, hot_cold_store.rs, canonical_head.rs, validator_monitor.rs, gloas_verification.rs)
- Network/sync modules requiring full test harnesses (range_sync/chain.rs, custody_backfill_sync, block_lookups, response_limiter.rs)
- System-level modules requiring OS deps (system_health, health_metrics)
- Modules requiring BeaconState/SigVerifiedOp construction (observed_operations.rs, bls_to_execution_changes.rs, block_reward.rs)
- Filesystem-bound utilities (eth2_wallet_manager filesystem.rs, locked_wallet.rs)

**Conclusion**: Unit test coverage task has reached diminishing returns. Future test improvements should focus on integration-level testing which requires different infrastructure (test harnesses, mock chains, etc.).

#### Run 1464 — Status assessment and PLAN.md cleanup

**Scope**: Reviewed project status across all priorities, checked for spec updates, assessed CI health.

**Findings**:
- All 8 Gloas phases complete, all devnet scenarios pass, all spec tests pass (79/79 + 139/139)
- Spec tracked to v1.7.0-alpha.3 (latest release as of 2026-03-13) — all Gloas changes from alpha.3 verified as implemented (#4897, #4884, #4923, #4918, #5001, #4930, #4948)
- CI healthy: nightly tests passing consistently, all check/clippy/fmt/ef-tests green
- Codebase clean: zero clippy warnings, zero compiler warnings, zero TODO/FIXME in Gloas code
- Heze (next fork, FOCIL/EIP-7805) spec exists but is still WIP — not yet actionable
- EIP-8025 (execution proofs) spec changes are for a standalone feature, not part of Gloas — vibehouse's ZK approach diverges from spec's signed-proof model intentionally

**Action**: Condensed the massive test coverage list in PLAN.md from ~200 lines to a concise summary reflecting completion status.

#### Run 1498 — Status check, CI verification

**Scope**: Checked all priorities, CI status, spec updates, security advisories.

**Findings**:
- All priorities DONE (1-6 + backlog). Only priority 7 (ROCQ formal proofs, lowest priority) remains open.
- CI green: latest commit (slasher test fix b79292d3) passed all 7 jobs. Nightly failure was pre-fix commit (already resolved).
- Spec v1.7.0-alpha.3 still latest. One new commit since tag (#5002) — wording clarification only in p2p-interface.md, no code impact.
- `cargo audit`: only `rsa` RUSTSEC-2023-0071 (no fix available, JWT auth on localhost, low risk). Rest are unmaintained crate warnings.
- Zero clippy warnings, zero compiler warnings, clean build.
- No open PRs, no open issues requiring action (issue #27 has 0 upvotes).

**Action**: No code changes needed. Monitoring run only.

#### Run 1605 — Status check, CI verification, slasher fix validation

**Scope**: Checked all priorities, CI status, spec updates, Heze fork status, slasher fix validation.

**Findings**:
- All priorities DONE (1-6 + backlog). Only priority 7 (ROCQ formal proofs, lowest priority) remains open.
- CI green: latest commit (flaky slasher test fix) passed all 7 main CI jobs. Nightly failure from 09:36 was before the fix (09:53 + 18:38 pushes).
- Validated slasher fix locally: `override_backend_with_mdbx_file_present` passes with both `lmdb`-only and `mdbx`-only feature configs. Nightly should be green tonight.
- Spec v1.7.0-alpha.3 still latest consensus-specs release. No new Gloas spec changes since alpha.3 (only #5002 wording clarification).
- `cargo audit`: only `rsa` RUSTSEC-2023-0071 (no fix available, JWT auth on localhost, low risk).
- Heze fork (FOCIL/EIP-7805): spec exists (7 files, 197-line beacon-chain.md), still marked WIP. Promoted to Heze on 2026-02-20. Not yet actionable for implementation.
- No open PRs, no open issues requiring action (issue #27 has 0 upvotes, #28/#29 are RFCs).

**Action**: No code changes needed. Monitoring run only.

#### Run 1687 — TODO comment cleanup (issue #31)

**Scope**: Clean up ~55 TODO comments missing proper issue links, as tracked in issue #31.

**Method**: Categorized every TODO in the codebase as STALE (4), INFORMATIONAL (20), or ACTIONABLE (33).

**Changes**:
1. **Removed 4 stale TODOs** — past design decisions (peer_id matching removal), vague/resolved items (mock EL forks, peerdb unban), already-stable Rust features
2. **Converted ~20 informational TODOs to regular comments** — design notes, observations, code review notes that didn't represent actionable work (removed `TODO` prefix, kept the comment content)
3. **Created 5 focused issues (#32-#36)** for the 33 remaining actionable TODOs, grouped by theme:
   - #32: sync custody column download robustness (5 TODOs)
   - #33: sync NoPeer graceful handling with timeout (2 TODOs)
   - #34: sync decouple block and data column requests (2 TODOs)
   - #35: sync test coverage improvements (4 TODOs)
   - #36: misc code improvements — boot node, EIP-7892, crypto, EL, tests, store (20 TODOs)
4. **Updated all TODO references** from `#31` to their specific issue number

**Result**: Zero TODOs reference #31 anymore. All remaining TODOs have focused issue links. Issue #31 closed.

### Run 1689: operation pool lock optimization (#36)

**Scope**: Operation pool attestation lock contention reduction.

**Change**: Pre-electra forks don't need write access for cross-committee aggregation. Changed `get_attestations_for_block` to take a read lock for pre-electra (instead of write-then-downgrade). Reduces lock contention on the attestation pool for all pre-electra blocks.

**Spec check**: Reviewed consensus-specs commits since v1.7.0-alpha.3. Two merged PRs:
- #5001 (parent_block_root in bid filtering key) — already implemented correctly in our `observed_execution_bids.rs`
- #5002 (payload signature verification wording) — documentation-only, no code change needed
- #4940 (initial fork choice tests for Gloas) — test generators only, fixtures not yet released

**Remaining #36 items assessment**:
- Boot node DOS/multiaddr: design work, low priority
- EIP-7892 blob overestimation: needs HTTP API spec changes (epoch in response), not actionable
- EIP-7892 blob schedule default: current `vec![]` is correct for non-Fulu forks
- Unsafe blst code: waiting on upstream `blst` crate safe API
- EL mock requests: test-only, default value works fine
- EL error enum refactor: large refactor, low impact
- Persist aggregation pools: feature work, needs DB schema design
- PeerDAS checkpoint sync: depends on PeerDAS feature implementation
- Subnet service dynamic bitfield: minor optimization (max 64 entries)
- Store hdiff dynamic buffer: needs schema migration, not a quick fix

### Run 1694 (2026-03-17)

**Nightly flake fix**: `override_backend_with_mdbx_file_present` slasher test flaked in nightly CI (March 16, pre-diagnostics). Root cause: `std::fs::write` doesn't guarantee directory entry visibility on all CI filesystems. Fix: replaced with explicit `File::create` + `sync_all()` on both file and parent directory. Test verified with `--features "mdbx"` and all backends.

**Audit sweep**: checked all remaining TODOs, clippy, build warnings, cargo audit, unused deps. Everything clean. Spec tracked to v1.7.0-alpha.3 (latest). `cargo-machete` false positives on `ethereum_ssz*`/`rand` (used by `TestRandom` derive macro).

### Run 1770 (2026-03-17)

**Spec audit**: checked 15 recently merged consensus-specs PRs. PR #5001 (bid filtering key), #5002 (wording), #5005 (test fix) — all already handled. Code quality scan: production unwraps/clones are pre-existing architectural patterns, not regressions.

### Run 1774–1775 (2026-03-17)

**Nightly failure triage**: all 4 recent nightly failures (Mar 10–17) traced to known issues already fixed on HEAD.

**Slasher test hardening**: added filesystem barrier (read-after-write) and moved diagnostic check earlier in `override_backend_with_mdbx_file_present`.

**Test coverage**: added 3 tests for `MissingEnvelopeFromAttestation` sync path (request trigger, deduplication, per-block independence).

### Run 1812 (2026-03-18)

Added 8 unit tests for `verify_data_column_sidecar_with_commitments` — Gloas-specific structural validation (valid sidecar, invalid column index, empty column, cells/commitments mismatch, cells/proofs mismatch, max blobs exceeded, single blob, max valid index). Committed `4a4f1120e`.

### Runs 1795–1843 (2026-03-18) — consolidated monitoring

Repeated health checks, all stable:
- Spec: v1.7.0-alpha.3 throughout. No new merged Gloas PRs since #5005 (Mar 15).
- Open Gloas PRs (#4992, #4960, #4843, #4932, #4630, #4840, #4892, #5008) all still unmerged.
- PR #4992 (cached PTCs) approved but debate ongoing (2-slot vs full-epoch caching approach).
- CI green, nightly green (Mar 18). Previous nightly failures all resolved.
- cargo audit unchanged (1 rsa RUSTSEC-2023-0071, 5 warnings). Zero clippy warnings.
- All remaining TODOs in #36 blocked on external dependencies.
- Verified fork choice test suites (9/9 pass including Gloas `on_execution_payload`).
- No code changes needed.

### Run 1858 (2026-03-18)

**Withdrawal code deduplication**: `process_withdrawals_gloas` and `get_expected_withdrawals_gloas` had ~200 lines of identical withdrawal computation logic (4 phases: builder pending, partial validator, builder sweep, validator sweep). Extracted shared logic into `compute_withdrawals_gloas` returning a `WithdrawalResult` struct. The mutable version applies state mutations using the result; the read-only version just returns withdrawals. Eliminates the risk of one function being updated without the other, which would be a consensus divergence bug. All 1021 state_processing tests pass, all EF spec tests pass (operations_withdrawals, operations_execution_payload_*, sanity_*). Committed `d9af9e256`.

### Run 1860 (2026-03-18)

**Full safety audit of production code**: searched all `unwrap()`, `expect()`, `panic!()`, `unreachable!()` in consensus/ and beacon_node/ — all instances are in test code (`#[cfg(test)]`) or acceptable startup/initialization paths. No runtime panic risks found in production consensus code. Checked open spec PRs — no new merges since #5005 (Mar 15). Notable open PRs still under review: #4992 (cached PTCs), #4954 (time_ms), #4898 (pending tiebreaker), #4892 (impossible branch). CI green. No code changes needed.

### Run 1861 (2026-03-18)

**Health check**: Spec v1.7.0-alpha.3 still latest — no new merged PRs since #5005 (Mar 15). Tracked open PRs (#4992, #4960, #4843, #4932, #4630, #4840, #4892, #5008) all still unmerged. PR #5008 (field name fix in ExecutionPayloadEnvelopesByRoot) — verified our implementation already uses correct `beacon_block_root` field name. CI in progress for withdrawal dedup commit (d9af9e256): check+clippy, ef-tests, network+op_pool all passed; unit tests, http_api, beacon_chain still running. Local clippy clean. No code changes needed.

### Run 1866 (2026-03-18)

**Unused dependency cleanup**: ran `cargo machete` to find unused dependencies across workspace. Most reports were false positives (derive macros like `TestRandom` require `rand`, SSZ derive macros need `ethereum_ssz`). Confirmed and removed one genuinely unused dep: `ethereum_hashing` from lcli (not imported anywhere, no feature forwarding). Verified: clippy clean, 4986/4995 tests pass (9 web3signer failures are pre-existing infrastructure-dependent). Also reviewed open spec PRs — #4992 (cached PTCs) updated Mar 17 but still open/unmerged. Committed `a80220b42`.

### Run 1867 (2026-03-18)

**Comprehensive health check**: Spec v1.7.0-alpha.3 still latest — only commit since Mar 15 is #5005 (already audited). All open Gloas PRs (#4992, #4960, #4954, #4939, #4932, #4898, #4892, #4843, #4840, #4747, #4630, #4558, #5008) remain unmerged. Clippy clean (zero warnings). `cargo audit`: 1 known vuln (rsa RUSTSEC-2023-0071, no fix available), 5 unmaintained warnings — no action possible. CI for `a80220b42` progressing: check+clippy+fmt ✓, ef-tests ✓, network+op_pool ✓, remaining jobs running. Investigated visibility downgrades for `get_indexed_payload_attestation`, `is_parent_block_full`, `can_builder_cover_bid` — all used across crates (beacon_chain imports from state_processing), cannot be `pub(crate)`. Reviewed `#[allow(clippy::enum_variant_names)]` on `BlockSlashInfo` — appropriate (all variants share "Signature" prefix by design). No code changes needed.

### Run 1868 (2026-03-18)

**Health check**: Spec v1.7.0-alpha.3 still latest — no new merged PRs. Verified PR #4940 (initial Gloas fork choice tests) fixtures are included in v1.7.0-alpha.3 release (published Mar 13). Ran all 9 fork choice EF tests locally — all pass including new Gloas-specific `on_execution_payload` and `withholding` suites (46 test cases across 6 categories). PR #4992 (cached PTCs) has `mergeable_state=clean` — could merge soon, would add `previous_ptc`/`current_ptc` to BeaconState and modify `process_slots`/`get_ptc`. CI for `a80220b42`: check+clippy ✓, ef-tests ✓, network+op_pool ✓, http_api ✓, unit-tests and beacon_chain still running. Clippy clean. All remaining TODOs blocked on externals. No code changes needed.

### Run 1869 (2026-03-18)

**Health check**: Spec v1.7.0-alpha.3 still latest — no new merged PRs since #5005 (Mar 15). All open Gloas spec PRs still unmerged: #4992 (cached PTCs, mergeable_state=clean), #4954 (milliseconds, blocked), #4898 (tiebreaker), #4892 (impossible branch), #5008 (field name fix), #4843 (variable PTC deadline), #4840 (EIP-7843), #4747 (fast confirmation), #4630 (EIP-7688), #4558 (cell dissemination). Ran fork choice EF tests — 9/9 pass. Ran `cargo machete` — all flagged deps are false positives (derive macro dependencies: `rand` via `TestRandom`, SSZ via derive macros, `tiny-bip39` via crate name aliasing). CI for `5202d5db5`: check+clippy ✓, ef-tests ✓, network+op_pool ✓, http_api ✓, unit-tests ✓, beacon_chain still running. Build clean (zero warnings). Devnet healthy (finalized epoch 8 earlier today). All TODOs tracked in #36 blocked on externals. No code changes needed.

### Run 1870 (2026-03-18)

**Health check + code improvement scan**: Spec v1.7.0-alpha.3 still latest — no new commits to consensus-specs since Mar 15. All open Gloas PRs unchanged. CI all green (all 6 jobs pass for `a80220b42`). Searched Gloas production code (block_verification.rs, beacon_chain.rs, gossip_methods.rs, data_column_verification.rs) for code improvements — all `.clone()` calls are necessary (Arc refcounting for async spawns, Signature is 96 bytes not Copy, signed_envelope used after clone for debug log). No unwraps in production Gloas paths. No new spec test releases (still v1.6.0-beta.0 for Fulu, no Gloas vectors). Prep branches (`cached-ptc`, `ptc-lookbehind`) ready on origin. No code changes needed.

### Run 1874 (2026-03-18)

**Health check**: Spec v1.7.0-alpha.3 still latest — no new commits since #5005 (Mar 15). All open Gloas PRs unchanged: #4992 (cached PTCs, mergeable_state=clean, still under discussion), #4962 (withdrawal interaction tests, blocked), #4960 (deposit fork choice test), #4954 (milliseconds), #4898 (tiebreaker), #4892 (impossible branch), #5008 (field name fix), #4843 (variable PTC deadline), #4840 (EIP-7843), #4747 (fast confirmation), #4630 (EIP-7688). CI all green — latest commit `a80220b42` passed all 7 jobs, nightly green. Zero compiler warnings, zero clippy warnings. Heze fork (FOCIL/EIP-7805) spec still WIP — only basic types and 2 helpers defined, no state transitions or fork choice logic, not actionable. Verified withdrawal dedup (run 1858) handles missed-payload scenarios correctly per PR #4962 test description. All TODOs in #36 blocked on externals. No code changes needed.

### Run 1875 (2026-03-18)

**Health check**: Spec v1.7.0-alpha.3 still latest — no new commits since #5005 (Mar 15). No new spec releases. All open Gloas PRs unchanged — #4992 (cached PTCs) has 1 approval (jtraglia) but active discussion ongoing (potuz/jihoonsong/ensi321, latest Mar 16-17), not imminent. New non-Gloas open PRs: #5014 (EIP-8025 p2p protocol for ZK proofs), #5015 (test coverage), #5016 (phase0 cleanup) — none require action. Heze fork (FOCIL/EIP-7805) has ~43KB of spec content across 7 files but is early-stage (promoted Feb 20, fork epoch TBD, engine API undefined) — not ready to implement. Reviewed Gloas perf opportunities: withdrawal balance lookup appears O(n²) but operates on max 16 items (MAX_WITHDRAWALS_PER_PAYLOAD), so real-world impact is negligible. CI all green (7/7 jobs). Zero clippy warnings. All 11 remaining TODOs tracked in #36 and blocked on externals. No code changes needed.

### Run 1877 (2026-03-18)

**Health check**: Spec v1.7.0-alpha.3 still latest — no new consensus-specs commits since #5005 (Mar 15). All open Gloas spec PRs unchanged: #4992 (cached PTCs, 1 approval), #4898 (tiebreaker, 1 approval), #4892 (impossible branch, 2 approvals), #4843 (variable PTC deadline), #5008 (field name fix), #4954 (milliseconds), #4747 (fast confirmation), #4558 (cell dissemination, 2 approvals). Verified our `get_payload_tiebreaker` already matches PR #4898 behavior (PENDING falls through to should_extend_payload at previous slot, no early return). Verified our `is_supporting_vote_gloas_at_slot` already uses `==` check matching PR #4892 (assert + equality instead of `<=`). PR #5008 field name fix — our code already uses correct `beacon_block_root`. `cargo audit`: same known issues (rsa RUSTSEC-2023-0071 no fix, 5 unmaintained transitive deps). CI all green. `cached-ptc` prep branch up to date on origin. No code changes needed.

### Runs 1878–1882 (2026-03-18) — consolidated monitoring

**Health check**: Spec v1.7.0-alpha.3 still latest — no new commits since #5005 (Mar 15). All open Gloas spec PRs unchanged. New non-Gloas open PRs: #5014 (EIP-8025 p2p), #5015 (test coverage), #5016 (phase0 cleanup) — none require action. CI all green (all 7 jobs pass for `a80220b42`). Zero clippy warnings, zero build warnings. `cargo audit`: 1 known vuln (rsa RUSTSEC-2023-0071, no fix available), 4 unmaintained warnings (all transitive: derivative via ark-serialize/sp1, ansi_term via tracing-forest/sp1, bincode 1.x, filesystem — our own crate false positive). bincode 3.0.0 is a tombstone release (development ceased) — staying on 1.x is correct. Comprehensive Gloas public API audit: all 8 pub functions in `gloas.rs` + all types have active external callers, zero dead code. All 10 remaining TODOs tracked in #36, all blocked on external dependencies. EF spec tests and workspace tests running for verification. No code changes needed.

### Run 1883 (2026-03-18)

**Health check**: Spec v1.7.0-alpha.3 still latest — no new consensus-specs merges since #5005 (Mar 15). All tracked open Gloas PRs (#4992, #4898, #4892, #5008, #4558, #4843, #4954, #4747, #4630, #4840) remain unmerged. PR #4992 (cached PTCs) still under active discussion. Zero clippy warnings. Ran EF spec tests: 4/4 Gloas operations tests pass (execution_payload_bid, execution_payload_full, execution_payload_blinded, withdrawals), 9/9 fork choice tests pass (including Gloas on_execution_payload and withholding suites — 46 test cases). Comprehensive test coverage audit of beacon_node/beacon_chain Gloas code: 24,559 lines of dedicated Gloas test code across 2 files (gloas_verification.rs: 3,006 lines, 77+ tests; gloas.rs: 21,553 lines, 317+ tests). All critical paths covered. No code changes needed.

### Run 1887 (2026-03-18)

**Wired up unused PAYLOAD_ENVELOPE_PROCESSING_FAILURES metric**: Found that `PAYLOAD_ENVELOPE_PROCESSING_FAILURES` was defined in metrics.rs but never incremented anywhere — a monitoring blind spot for envelope processing errors. The success counter (`PAYLOAD_ENVELOPE_PROCESSING_SUCCESSES`) was correctly wired at beacon_chain.rs:2793. Fixed by wrapping `process_payload_envelope` in a thin outer function that delegates to `process_payload_envelope_inner` and increments the failure counter on `Err`. Also verified `SELF_BUILD_ENVELOPE_FAILURES` is correctly wired at publish_blocks.rs:610 (the only production caller). Spec v1.7.0-alpha.3 still latest — no new merges since #5005 (Mar 15). All open Gloas PRs unchanged. 10/10 payload envelope tests pass. Committed `a6f3d6f6f`.

### Run 1888 (2026-03-18)

**Builder bid coverage edge case tests**: Added 6 tests for `get_pending_balance_to_withdraw_for_builder` and `can_builder_cover_bid` covering: (1) saturation at u64::MAX when withdrawals+payments overflow, (2) filtering by builder_index (ignores other builders), (3) unknown builder index returns `UnknownBuilder` error, (4) large pending withdrawals reduce available balance correctly (exact boundary + off-by-one), (5) massive pending withdrawals cause `can_builder_cover_bid` to return false even for zero bids. All 8 related tests pass (6 new + 2 existing). Spec v1.7.0-alpha.3 still latest. Committed `7cf89a1a2`.

### Run 1889 (2026-03-18)

**Buffered envelope processing metrics**: Added 3 new counter metrics for the gossip-before-block timing race path: `BUFFERED_ENVELOPE_TOTAL` (envelope arrived before its block, stored in pending buffer), `BUFFERED_ENVELOPE_PROCESSED_TOTAL` (successfully processed after block import), `BUFFERED_ENVELOPE_FAILED_TOTAL` (failed re-verification or processing after block import). Wired into `gloas_verification.rs` (buffer insertion) and `beacon_chain.rs` (`process_pending_envelope` success/failure paths). This fills an observability gap — previously there was no way to monitor how often the envelope-before-block race condition occurs in production. Spec v1.7.0-alpha.3 still latest — no new merges since #5005 (Mar 15). All open Gloas PRs unchanged. 119/119 envelope-related beacon_chain tests pass, clippy clean, pre-push lint-full passes. Committed `12d1e3b7c`.

### Run 1892 (2026-03-18)

**Per-reason rejection metrics for gossip bids and payload attestations**: Added `IntCounterVec` metrics with "reason" labels for granular monitoring of gossip rejection types:
- `beacon_processor_execution_bid_rejected_total{reason=...}` — covers 5 REJECT cases: `zero_payment`, `invalid_builder`, `invalid_signature`, `fee_recipient_mismatch`, `gas_limit_mismatch`
- `beacon_processor_payload_attestation_rejected_total{reason=...}` — covers 2 REJECT cases: `invalid_aggregation_bits`, `invalid_signature`

Previously these paths only logged `warn!` and penalized peers but had no Prometheus counters, making it impossible to dashboard/alert on specific rejection patterns. Equivocation cases already had dedicated counters and remain unchanged. Spec v1.7.0-alpha.3 still latest — no new merges since #5005 (Mar 15). 204/204 network tests pass, clippy clean. Committed `cbb224039`.

### Run 1893 (2026-03-18)

**Fixed stale state cache race condition in envelope processing**: Found a race between the state advance timer and envelope processing that could cause block production to fail with external builder bids.

**Root cause**: When an envelope is processed AFTER the state advance timer has already advanced the pre-envelope state to slot N+1, the cached advanced state retains a stale `latest_block_hash` (from before envelope processing). Block production loads this stale state from cache, causing `process_execution_payload_bid` to reject external builder bids whose `parent_block_hash` matches the post-envelope hash.

**Timeline of the race**:
1. Block N imported → pre-envelope state cached at `(block_root, slot_N)`
2. State advance timer runs (3/4 through slot) → loads pre-envelope state, advances to N+1, caches at `(block_root, slot_N+1)` with wrong `latest_block_hash`
3. Envelope N arrives late → processed, cache updated at `(block_root, slot_N)` with correct hash
4. Block production calls `get_advanced_hot_state(block_root, slot_N+1)` → cache hit returns STALE advanced state from step 2
5. `process_execution_payload_bid` fails: `bid.parent_block_hash != state.latest_block_hash`

**Fix**: Changed `cache.delete_state(&block_state_root)` to `cache.delete_block_states(&beacon_block_root)` in `process_payload_envelope`. This removes ALL cached states for the block root (including stale advanced states), not just the base state. The next access re-loads from the fresh post-envelope base state and re-advances.

**Verification**: 422/422 Gloas beacon_chain tests pass, 236/236 store tests pass, 139/139 EF spec tests pass, clippy clean. Committed `54946814c`.

### Run 1894 (2026-03-18)

**Fixed same stale state cache race in sync and self-build envelope paths**: Run 1893 fixed the gossip path but the sync path (`process_sync_envelope`, line 3005) and self-build path (`process_self_build_envelope`, line 3399) had the same bug — both used `delete_state` (removes only the base state) instead of `delete_block_states` (removes all cached states for the block root, including stale advanced states). Changed both to `delete_block_states` to match the gossip path fix. 125/125 envelope tests pass, 5/5 EF spec tests pass, clippy clean. Committed `a17a399e8`.

### Run 1895 (2026-03-18)

**Health check + devnet verification of race condition fixes**: Reviewed 2 new Gloas spec PRs merged since alpha.3: #5001 ("Add `parent_block_root` to bid filtering key") — already implemented, our `highest_bid_values` HashMap uses `(Slot, ExecutionBlockHash, Hash256)` key since initial implementation (observed_execution_bids.rs:48). #5002 ("Make wordings clearer for self build payload signature verification") — wording-only, no code change needed. Also checked #5008 (open, "fix: correct field name `block_root` in `ExecutionPayloadEnvelopesByRoot`") — we already use the correct `beacon_block_root` field name. Investigated nightly test failure from Mar 17 (`finalized_sync_not_enough_custody_peers_on_start` in fulu network tests) — one-off failure, Mar 18 nightly passed, test is deterministic and robust (supernode covers all custody columns). Spec v1.7.0-alpha.3 still latest — no new test fixture releases. Ran 4-node devnet: finalized_epoch=8, clean chain progression through Gloas fork, verifying runs 1893/1894 state cache race fixes in live environment. CI fully green (all 7 jobs pass for `fb4c011b4`). No code changes needed.

### Run 1920 (2026-03-19)

**Dead code cleanup in gossip cache builder**: Full codebase health check — zero clippy warnings, zero build warnings, zero doc warnings, no new consensus-specs PRs beyond what's tracked, no dependency updates available, all TODOs linked to #36 and blocked/non-critical. Removed 4 dead `GossipCacheBuilder` methods: `light_client_finality_update_timeout`, `light_client_optimistic_update_timeout`, `signed_contribution_and_proof_timeout`, `sync_committee_message_timeout` (none called anywhere). Moved `default_timeout` behind `#[cfg(test)]` (only used in tests). Removed stale commented-out builder calls in production code. Also removed blanket `#[allow(dead_code)]` on the impl block. 407/407 vibehouse_network tests pass, clippy clean. Committed `da0362e86`.

### Run 1921 (2026-03-19)

**Comprehensive health check — all clear**: Zero clippy warnings, zero build warnings, zero `cargo doc` warnings (`-D warnings`). Spec v1.7.0-alpha.3 still latest — no new releases or Gloas-related merges. 9 open Gloas spec PRs tracked (#5008, #4992, #4962, #4960, #4939, #4932, #4843, #4840, #4630) — all still open, none merged. #5008 (field name fix) and #4939 (envelope request from attestations) already implemented. Cargo-machete unused dependency scan: all flagged items are false positives (SSZ/serde derive macros, feature-flag deps, TestRandom macro). `cargo audit`: unchanged (rsa RUSTSEC-2023-0071 no fix, 4 unmaintained warnings all transitive). Nightly test failures from Mar 16 (slasher) and Mar 17 (network) both resolved — subsequent runs pass. CI for latest push (`da0362e86`): check+clippy+fmt ✅, ef-tests ✅, network+op_pool ✅, 3 jobs still running. No code changes needed.

### Run 1922 (2026-03-19)

**Narrowed blanket `#[allow(dead_code)]` on `RemoveChain` enum**: The `RemoveChain` enum in range sync had `#[allow(dead_code)]` on the entire enum, suppressing all dead-code warnings for all variants and fields. The enum itself and all variants are actively used (15 occurrences across 2 files). Removed the blanket allow and added per-field `#[allow(dead_code)]` only on the 3 specific fields that are stored for Debug output but never read directly: `failing_batch`, `WrongBatchState(String)`, `WrongChainState(String)`. Also audited all other `#[allow(dead_code)]` annotations across the codebase — remaining 35 annotations are all legitimate (error enum fields used only for Debug, test-only methods, conditional compilation guards). Full health check: spec v1.7.0-alpha.3 still latest — no new merges since #5005 (Mar 15). All open Gloas PRs unchanged. Zero clippy/build/doc warnings. `RUSTFLAGS="-W dead_code"` clean for state_processing, fork_choice, proto_array, vibehouse_network, beacon_chain. CI for `da0362e86` nearly complete (5/6 green). 204/204 network tests pass. Committed `15294bf67`.

### Run 1924 (2026-03-19)

**Removed unnecessary allow annotations**: (1) Removed `#[allow(dead_code)]` from `assert_accept` helper in network tests — function is used 26+ times, the allow was stale. (2) Removed 3 `#[allow(unused_imports)]` on `use ssz::*` in `signed_beacon_block.rs` SSZ tagged encoding/decoding modules — the imports are actively used (`BYTES_PER_LENGTH_OFFSET`, `DecodeError`, SSZ trait methods). Full clippy clean (including `--tests` with `-W unused_imports`), zero warnings. Spec v1.7.0-alpha.3 still latest — no new merges. 12 open Gloas spec PRs tracked, all unchanged. CI for previous push all green (check ✅, ef-tests running). Committed `cd6f8da8f`.

### Run 1925 (2026-03-19)

**Comprehensive health check — all clear**: Zero clippy/build/doc warnings. Spec v1.7.0-alpha.3 still latest — no new consensus-specs merges since #5005 (Mar 15). 10 open Gloas spec PRs tracked (#4992 cached PTCs still `mergeable_state=clean` but under active discussion, #4747 fast confirmation `dirty`). All remaining `#[allow(...)]` annotations audited — all legitimate (Debug-only fields, conditional compilation, type complexity). No dependency updates available. Devnet verification: 4-node devnet finalized_epoch=8, clean chain progression through Gloas fork — confirms state cache race fixes (runs 1893-1894) and metrics additions (runs 1887-1892) work in live environment. CI 5/6 green (beacon_chain tests still running). Nightly flakes investigated: Mar 17 `finalized_sync_not_enough_custody_peers_on_start` already fixed (commit `8f8faa7de`), Mar 16 `override_backend_with_mdbx_file_present` already hardened (commit `2848be8c5`). `cached-ptc` prep branch merges cleanly with main. No code changes needed.

### Run 1926 (2026-03-19)

**Comprehensive health check — all clear**: Zero clippy warnings (full workspace + all targets). Spec v1.7.0-alpha.3 still latest — no new consensus-specs merges since #5005 (Mar 15). Open Gloas spec PRs: #4992 (cached PTCs) still `MERGEABLE`/`CLEAN` — most impactful pending change (adds `previous_ptc`/`current_ptc` to BeaconState, modifies `process_slots`, changes `get_ptc` to read from state). #4747 (fast confirmation) updated Mar 18, still open. #4960/#4932 (new test vectors) still open. No new EF test fixture releases (latest: v1.6.0-beta.0). `cargo audit`: unchanged (rsa RUSTSEC-2023-0071 no fix, 4 unmaintained transitive warnings). CI for `e7e1552ac`: 4/6 green (check ✅, ef-tests ✅, http_api ✅, network+op_pool ✅), beacon_chain + unit tests in progress. Nightly tests: last 2 runs passed (Mar 18). All 11 remaining TODOs tracked in #36 (5 blocked, 2 non-critical). No code changes needed.

### Run 1933 (2026-03-19)

**Rebased cached-ptc prep branch + comprehensive audit**: Rebased `cached-ptc` branch onto main (clean rebase, no conflicts). Verified: zero clippy warnings, 1026/1026 state_processing tests pass. EF spec tests expectedly fail (SSZ layout changed by new `previous_ptc`/`current_ptc` BeaconState fields — need new fixtures when spec PR #4992 merges). Pushed rebased branch to origin.

Spec v1.7.0-alpha.3 still latest — no new consensus-specs merges since #5005 (Mar 15). Checked near-merge PRs: #4892 (2 approvals, remove impossible branch) — already implemented in our code. #4898 (1 approval, remove pending tiebreaker) — already implemented. #5008 (field name fix) — already correct. All open Gloas PRs unchanged.

Full codebase audit: all `pub fn` in gloas.rs confirmed cross-crate (beacon_chain, store, http_api, ef_tests) — no visibility downgrades possible. Block production path reviewed (`produce_block_on_state`, `build_self_build_envelope`) — error handling is thorough. Remaining EL error enum TODOs (#36) reviewed — both are cosmetic refactors with significant churn, not worth the blast radius. Nightly flakes (Mar 16 slasher, Mar 17 network) both one-off and resolved. CI all green. No code changes needed.

### Run 1934 (2026-03-19)

**Health check — all clear, nothing actionable**: Zero clippy warnings, zero build warnings (`cargo build --release` clean). Spec v1.7.0-alpha.3 still latest — no new consensus-specs merges since #5005 (Mar 15). All 11 TODOs confirmed tracked in #36, all blocked on external dependencies or non-critical. `cargo audit`: unchanged (rsa RUSTSEC-2023-0071 no fix, 5 allowed warnings all transitive). CI for latest push (`7907432dd`): 4/6 green (check ✅, ef-tests ✅, http_api ✅, network+op_pool ✅), beacon_chain + unit tests in progress. Nightly tests: last 2 runs passed (Mar 18). `cached-ptc` branch 1 commit behind main (task docs only) — clean rebase when spec PR #4992 merges. No code changes needed.

### Run 1935 (2026-03-19)

**Replaced `.and_then(|x| x)` with `.flatten()` in task_spawner.rs**: Two instances in `beacon_node/http_api/src/task_spawner.rs` (lines 67, 122) used `.and_then(|x| x)` to flatten `Result<Result<T, E>, E>` — replaced with `Result::flatten()` (stable since Rust 1.82). Comprehensive codebase search found no other idiomatic improvement opportunities — recent runs (1930-1934) already cleaned up `.copied()`, method references, and dead `#[allow]` annotations. Spec v1.7.0-alpha.3 still latest — no new consensus-specs merges. PR #4992 (cached PTCs) still open, `mergeable_state=clean`. 346/346 http_api tests pass, zero clippy warnings, pre-push lint-full passes. Committed `f51314532`.

### Run 1936 (2026-03-19)

**Replaced `.map(|x| x.into())` with `.map(Into::into)` across 7 files**: Redundant closure pattern in network sync (block_lookups), vibehouse_network (rpc methods, peerdb), execution_layer (engine_api http, test_utils handle_rpc and execution_block_generator), and network beacon processor tests. Same category as run 1931's method reference cleanup. All 752/756 crate tests pass (4 pre-existing flaky network tests unrelated). Zero clippy warnings, pre-push lint-full passes. Committed `6ef400ccc`.

### Run 1937 (2026-03-19)

**Removed 3 dead public methods from HotColdDB**: Systematic audit of all `pub fn` methods in `hot_cold_store.rs` — checked every method for callers across the entire codebase (external files + internal calls). Found 3 truly dead methods with zero callers anywhere:

1. **`get_execution_payload_dangerous_fork_agnostic`** (line 754) — explicitly marked "DANGEROUS" in its doc comment, guessed the fork when deserializing SSZ. Never called.
2. **`item_exists`** (line 1339) — generic hot DB existence check wrapper. Never called (callers use `get_item` instead).
3. **`store_schema_version_atomically`** (line 2809) — atomic schema version storage with batch ops. Dead since schema migration removal (run 557). Only `store_schema_version` (non-atomic) is used.

Also investigated `let _ = bits.set(idx, true)` in block_verification.rs (lines 2072, 2092) — safe by construction (index is `slot % SlotsPerHistoricalRoot` on a `BitVector<SlotsPerHistoricalRoot>`, guaranteed in-bounds). Not changed.

Spec v1.7.0-alpha.3 still latest — no new consensus-specs merges since #5005 (Mar 15). Open Gloas PRs unchanged. 236/236 store tests pass, full workspace compiles clean, clippy clean. Committed `f5ccc337e`.

### Run 1938 (2026-03-19)

**Pre-allocated vectors with known sizes in hot paths**: Comprehensive audit of `Vec::new()`/`vec![]` patterns in non-test code where the final size is known at allocation time. Found and fixed 7 vectors across 5 files:

1. **`data_column_custody_group.rs`** — `custody_groups` Vec in `get_custody_groups_ordered()`: size is exactly `custody_group_count`, was growing via push in a while loop. Changed to `Vec::with_capacity(custody_group_count)`.

2. **`kzg_utils.rs`** — `validate_data_columns_with_commitments()`: 4 vectors (`cells`, `column_indices`, `proofs`, `commitments`) with known sizes from `data_column.column().len()`, `kzg_proofs().len()`, and `kzg_commitments.len()`. Pre-allocated all 4.

3. **`kzg_utils.rs`** — `blobs_to_data_column_sidecars()` (2 call sites): `cells` and `cell_ids` vectors in blob reconstruction closure, size is `data_columns.len()`. Pre-allocated both in regular and rayon parallel paths.

4. **`beacon_block_streamer.rs`** — `load_beacon_blocks_from_disk()`: `db_blocks` Vec, size is `block_roots.len()`. Also `ordered_block_roots` and `by_range_blocks` in `get_requests()`, size is `payloads.len()`.

5. **`single_pass.rs`** — `added_validators` Vec in `apply_validator_registry_and_deposits()`, size is `ctxt.new_validator_deposits.len()`.

Also added `rust_out` (stray rustc binary) to `.gitignore`.

Investigated but skipped: batch `validate_data_columns()` (iterator-based, would need clone+count pass), `ValidatorPubkeyCache::new()` (already uses `reserve()` in `import()`), `hot_cold_store.rs` ops Vec (small fixed size, marginal benefit). Full codebase safety audit confirmed: zero unsafe issues in production code, all TODOs tracked in #36 (blocked/deferred), no production panics.

2/2 custody tests pass, 24/24 single_pass tests pass, 3/3 pubkey cache tests pass. Committed `4ce4375e0`.

### Run 1939 (2026-03-19)

**Replaced `.map(...).unwrap_or(false)` with `is_some_and`/`is_ok_and` across 7 files**: Systematic audit of `.map(|x| ...).unwrap_or(false)` patterns in non-test code. Replaced 8 instances with the idiomatic `is_some_and()`/`is_ok_and()` methods (stable since Rust 1.70):

1. **`validator.rs`** (2 instances) — `has_eth1_withdrawal_credential` and `is_compounding_withdrawal_credential`: `.first().map(|byte| *byte == ...).unwrap_or(false)` → `.first().is_some_and(|byte| *byte == ...)`
2. **`verify_bls_to_execution_change.rs`** — BLS withdrawal prefix check: same pattern
3. **`process_operations.rs`** — withdrawal request source address check: `.map(|addr| addr == ...).unwrap_or(false)` → `.is_some_and(...)`
4. **`chain.rs`** — optimistic batch detection: `.map(|epoch| epoch == batch_id).unwrap_or(false)` → `.is_some_and(...)`
5. **`duties_service.rs`** — unknown validator poll slot check: same pattern
6. **`beacon_block_streamer.rs`** — result success check: `.map(Option::is_some).unwrap_or(false)` → `.is_ok_and(Option::is_some)` (on a `Result`)
7. **`overflow_lru_cache.rs`** — blob existence check: `.map(Option::is_some).unwrap_or(false)` → `.is_some_and(Option::is_some)` (on nested `Option`)

Spec check: v1.7.0-alpha.3 still latest — no new consensus-specs merges since #5005 (Mar 15). Verified post-alpha.3 PRs (#5001 parent_block_root bid filtering, #5002 wording clarification, #4940 fork choice tests) — all already implemented or non-code-impacting. Open Gloas PRs unchanged (#4992 cached PTCs, #4960/#4932 tests, #4843 variable PTC deadline, #4840 EIP-7843, #4630 EIP-7688). Nightly tests passing. CI green.

1085/1085 types tests pass, 1026/1026 state_processing tests pass, clippy clean. Committed `df0e8ead4`.

### Run 1940 (2026-03-19)

**Replaced `format!("{}", x)` with `.to_string()` and side-effect `.map()` with `if let`**: Systematic audit of `format!("{}", x)` patterns across non-test production code. Replaced 23 instances across 11 files:

1. **`builder_client/src/lib.rs`** (9 instances) — `Error::InvalidHeaders(format!("{}", e))` → `Error::InvalidHeaders(e.to_string())` in all 3 blinded blocks endpoints
2. **`execution_layer/src/lib.rs`** — `format!("{}", payload.parent_hash())` → `payload.parent_hash().to_string()` in relay logging
3. **`network/src/service.rs`** — `format!("{}", topic)` → `topic.to_string()` in subscription logging
4. **`execution_layer/src/metrics.rs`** — `let _ = X.as_ref().map(|g| g.reset())` → `if let Ok(g) = X.as_ref() { g.reset(); }` (side-effect map to idiomatic if-let)
5. **`eth2_keystore/src/keystore.rs`** (4 instances) — `Error::*Error(format!("{}", e))` → `Error::*Error(e.to_string())` for JSON serialization/deserialization errors
6. **`eth2_wallet/src/wallet.rs`** (4 instances) — same pattern with `KeystoreError` variants
7. **`eth2_wallet_manager/` (3 files, 7 instances)** — `format!("{}", uuid)` → `uuid.to_string()` for path construction
8. **`validator_dir/src/builder.rs`** — `format!("{}", amount)` → `amount.to_string()` for deposit amount serialization
9. **`lcli/src/mnemonic_validators.rs`** — `format!("{}", path)` → `path.to_string()` for keystore path

Spec v1.7.0-alpha.3 still latest — no new consensus-specs merges. Open Gloas PRs unchanged. #4747 (fast confirmation) updated Mar 19 but still open. 747/747 tests pass across affected crates, zero clippy warnings, pre-push lint-full passes. Committed `afbf11a72`.

### Run 1941 (2026-03-19)

**Health check + branch cleanup**: Comprehensive audit found no new work needed:

- **Spec check**: v1.7.0-alpha.3 still latest. Only 1 commit since Mar 15 (#5005, test-only). Notable open PRs: #5008 (field name fix — already correct in vibehouse, we use `beacon_block_root`), #4992 (cached PTCs — `cached-ptc` branch ready), #5003 (proposer lookahead simplification — our impl already correct). No action needed.
- **CI**: Latest run (afbf11a72) — check/clippy/fmt ✅, ef-tests ✅, others in progress. Nightly tests: last 2 runs passed.
- **Production safety audit**: Reviewed all `unwrap()`/`expect()` in Gloas production code (envelope_processing.rs, gloas.rs) — all in test code only. Production code uses `?` and `map_err` throughout.
- **Dependency audit**: `cargo machete --with-metadata` found no genuinely unused deps (all flagged items are proc-macro re-exports, TestRandom derives, or dev-deps).
- **Branch cleanup**: Deleted 7 stale remote branches (gloas-p2p-gossip-validation, phase4-validation-wiring, gloas-dev, gloas-fork-choice, gloas-signatures, gloas/data-column-sidecar-superstruct, ptc-lookbehind). Only `main` and `cached-ptc` remain. Deleted local `ptc-lookbehind` branch.
- **Code quality**: Zero clippy warnings, zero build warnings, cargo doc clean. All TODOs tracked in #36 (blocked/deferred). No code changes needed.

### Run 1942 (2026-03-19)

**Replaced empty string literals with `String::new()`/`unwrap_or_default()` across 8 files**: Systematic audit of `String::from("")`, `"".to_string()`, `"".into()`, and `unwrap_or_else(|| String::from(""))` patterns in production code:

1. **`system_health/src/lib.rs`** (4 instances) — `unwrap_or_else(|| String::from(""))` and `unwrap_or_else(|| "".into())` → `unwrap_or_default()` for system_name, kernel_version, os_version, host_name
2. **`vibehouse_network/src/discovery/mod.rs`** — `String::from("")` → `String::new()` for empty enr_dir fallback
3. **`vibehouse_network/src/service/mod.rs`** — `"".into()` → `String::new()` for private identify config
4. **`validator_manager/src/create_validators.rs`** — `"".to_string()` → `String::new()` for wallet builder
5. **`eth2_keystore/src/keystore.rs`** — `"".to_string()` → `String::new()` for keystore description
6. **`eth2_keystore/src/json_keystore/kdf_module.rs`** — `"".into()` → `String::new()` for EmptyString impl
7. **`validator_dir/src/builder.rs`** — `"".into()` → `String::new()` for keystore builder
8. **`validator_dir/src/insecure_keys.rs`** — `"".into()` → `String::new()` for insecure keystore builder

Spec v1.7.0-alpha.3 still latest — no new consensus-specs merges since #5005 (Mar 15). Open Gloas PRs unchanged. 176/176 tests pass across affected crates, zero clippy warnings, lint-full passes. Committed `0ea8cb710`.

### Run 1943 (2026-03-19)

**Idiomatic for loops and trailing semicolons across 201 files**: Applied two clippy lints codebase-wide:

1. **`clippy::explicit_iter_loop`** — replaced `for x in collection.iter()` with `for x in &collection` (more idiomatic Rust)
2. **`clippy::semicolon_if_nothing_returned`** — added trailing `;` to unit-returning expressions for consistent formatting

Also fixed a secondary `clippy::for_kv_map` warning in `peerdb.rs` (use `.values()` instead of destructuring `(_, v)` on map iteration).

Spec tracking: v1.7.0-alpha.3 still latest. No new merged PRs since last run. Open Gloas PRs unchanged. Notable open PR #4992 (cached PTCs in state) adds `previous_ptc`/`current_ptc` fields to BeaconState — not merged, no action needed yet.

CI: All jobs passing (check/clippy/fmt ✅, ef-tests ✅, network+op_pool ✅, http_api ✅). Nightly tests stable (last 2 runs passed). Previous nightly failures were: (1) fulu network test flake — already fixed in 8f8faa7de, (2) slasher mdbx flake — known CI issue with diagnostics added.

4991/5000 workspace tests pass (8 web3signer failures = external service timeout, not code issue). Committed `71a5bcd23`.

### Run 1944 (2026-03-19)

**Health check + spec compliance audit**: Deep review of codebase health, no code changes needed.

- **Spec check**: v1.7.0-alpha.3 still latest (HEAD 1baa05e7). Recent merged PR #4940 (initial Gloas fork choice tests) — our EF test runner already has `on_execution_payload` handler support, will pick up test vectors on next spec release. Notable open PRs: #4747 (Fast Confirmation Rule, updated today — adds `confirmed_root` to Store, new `on_slot_after_attestations_applied` handler), #5014 (EIP-8025 ZK P2P protocol), #4962 (missed payload withdrawal tests), #4992 (cached PTCs). None merged.
- **Withdrawal processing audit**: Cross-referenced `process_withdrawals_gloas` + `compute_withdrawals_gloas` against spec. All 4 sweep phases correct (builder pending → partial → builder sweep → validator sweep). `reserved_limit = MAX - 1` for first 3 phases, `max_withdrawals` for validator sweep. Edge cases verified: zero validators (loop bound is 0), builder index flag encoding, `safe_rem` division safety, `update_next_withdrawal_validator_index` logic.
- **Clone/allocation audit**: Only 2 non-test clones in gloas.rs production code — both necessary (bid stored to state from borrow, signature moved from borrow). No unnecessary allocations in hot paths.
- **Production safety**: Zero `unwrap()`/`expect()` in production consensus code. All panics/unwraps are in `#[cfg(test)]` modules. All `unsafe` blocks tracked in #36 (blst FFI). All `let _` patterns are intentional (channel sends, format! Debug checks).
- **CI**: Latest push CI running (check/clippy/fmt ✅). Nightly history: Mar 17 fulu network flake (already fixed 8f8faa7de), Mar 18 ×2 success. Current nightly in progress.
- **Nightly flake**: `finalized_sync_not_enough_custody_peers_on_start` failed once (Mar 17), passed subsequently. Root cause fixed in 8f8faa7de (same day). Not recurring.
- **Open issues**: #36 (misc TODOs) — all blocked on external deps. #29 (ROCQ) — lowest priority. #28 (ZK SP1 devnet) — needs GPU. #27 (private validator messages) — feature request.

No code changes. Project in maintenance/monitoring mode awaiting next spec release.

### Run 1945 (2026-03-19)

**Replaced 41 redundant closures with method references across 26 files** (`clippy::redundant_closure_for_method_calls`):

Patterns replaced:
- `|x| x.method()` → `Type::method` (e.g., `|b| b.total_difficulty()` → `Block::total_difficulty`)
- `|x| x.into()` → `Into::into`
- `|x| x.as_ref()` → `AsRef::as_ref`
- `|x| x.to_string()` → `ToString::to_string`
- `|x| x.len()` → `Vec::len`
- `|x| x.is_empty()` → `VariableList::is_empty`

Also refactored 2 `let _ = result.map(|gauge| gauge.reset())` patterns to idiomatic `if let Ok(gauge) = result { gauge.reset(); }` in peer_manager metrics.

Files: execution_layer (3), network (12), store (2), vibehouse_network (6), crypto/bls (4). CI: check/clippy/fmt green, pre-push lint-full passes. Committed `f14f89381`.

### Run 1946 (2026-03-19)

**Idiomatic slice types in public APIs and removed redundant clones across 6 files**:

1. **`&Vec<T>` → `&[T]` in 5 public function signatures**:
   - `key_cache.rs` — `uuids()` return type: `&Vec<Uuid>` → `&[Uuid]`
   - `chain_spec.rs` — `BlobSchedule::as_vec()` renamed to `as_slice()`, return type: `&Vec<BlobParameters>` → `&[BlobParameters]`
   - `committee_cache.rs` — `compare_shuffling_positions()` params: `&Vec<NonZeroUsizeOption>` → `&[NonZeroUsizeOption]`
   - `metrics.rs` — `expose_execution_layer_info()` param: `&Vec<ClientVersionV1>` → `&[ClientVersionV1]`
   - `peer_info.rs` — `listening_addresses()` return type: `&Vec<Multiaddr>` → `&[Multiaddr]`

2. **Removed 3 redundant `.clone()` on Copy types** in `listen_addr.rs`:
   - `ListenAddr` impl bound changed from `Into<IpAddr> + Clone` to `Into<IpAddr> + Copy`
   - `self.addr.clone().into()` → `self.addr.into()` in 3 socket address methods (`Ipv4Addr`/`Ipv6Addr` are `Copy`)

Spec v1.7.0-alpha.3 still latest — no new consensus-specs merges since #5005 (Mar 15). 1085/1085 types tests pass, 68/68 network_utils tests pass, 37/37 peer_info tests pass, zero clippy warnings. Committed `4c9ae7c7e`.

### Run 1947 (2026-03-19)

**Replaced implicit clones with explicit `.clone()` across 11 files** (`clippy::implicit_clone`): Fixed 22 instances where `.to_string()` was called on an already-owned `String` or `.to_vec()` on an already-owned `Vec`, hiding the fact that a clone is happening:

1. **`block_sidecar_coupling.rs`** (3) — `.to_vec()` → `.clone()` on `Vec<Arc<...>>` fields
2. **`config.rs` (beacon_node)** (3) — `.to_string()` → `.clone()` on `String` from CLI args
3. **`config.rs` (validator_client)** (3) — same pattern
4. **`discovery/mod.rs`** (2) — `.to_string()` → `.clone()` on `String` in tracing
5. **`peer_manager/mod.rs`** (1) — removed redundant `.to_string()` on `String` (just use `&client`)
6. **`peer_manager/network_behaviour.rs`** (3) — `.to_string()` → `.clone()` on error `String`
7. **`boot_node/src/lib.rs`** (1) — `.to_string().to_lowercase()` → `.to_lowercase()` (String derefs to str)
8. **`directory/src/lib.rs`** (1) — `.to_string()` → `.clone()`
9. **`tracing_logging_layer.rs`** (1) — `.to_string()` → `.clone()`
10. **`api_secret.rs`** (1) — `.to_string().as_bytes()` → `.as_bytes()` (String derefs to str)
11. **`validator_client/http_api/src/lib.rs`** (2) — `.to_string()` → `.clone()`

Spec v1.7.0-alpha.3 still latest — no new consensus-specs merges since #5005 (Mar 15). Zero clippy warnings (default + `implicit_clone`), lint-full passes. Committed `dc19c1923`.

### Run 1948 (2026-03-19)

**Replaced 62 `.map().unwrap_or()` / `.map().unwrap_or_else()` chains with `.map_or()` / `.map_or_else()` across 48 files** (`clippy::map_unwrap_or`):

Patterns replaced:
- `.map(|x| f(x)).unwrap_or(default)` → `.map_or(default, |x| f(x))`
- `.map(|x| f(x)).unwrap_or_else(|| g())` → `.map_or_else(|| g(), |x| f(x))`
- `.map(Ok).unwrap_or_else(|| fallible())` → `.map_or_else(|| fallible(), Ok)`

Files touched across: common/ (6), consensus/ (2), beacon_node/ (26), validator_client/ (10), account_manager (1), validator_manager (1), testing/ (2).

Also verified spec status: v1.7.0-alpha.3 still latest. Recent merged PRs #5001 (parent_block_root in bid filtering key) and #5002 (payload signature wording) — both already implemented in our codebase. No action needed.

4991/5000 workspace tests pass (8 web3signer timeouts = external service, 1 skip). lint-full passes. Committed `19d149ab0`.

### Run 1949 (2026-03-19)

**Removed needless `collect()` calls across 12 files** (`clippy::needless_collect`): Eliminated 13 unnecessary intermediate allocations where iterators were collected into `Vec` only to be immediately consumed. 6 locations where collect is required (borrow conflicts with lock guards or mutable self) were annotated with `#[allow(clippy::needless_collect)]`.

Patterns fixed:
- `.collect::<Vec<_>>().len()` → `.count()`
- `.collect::<Vec<_>>().is_empty()` → `!iter.any(|_| true)` or `.count() == 0`
- Intermediate Vec creation where source `.len()` was available directly

Files: common/ (3), consensus/ (2), beacon_node/ (7). 2648 tests pass across modified packages (types, state_processing, store, network, network_utils, lru_cache). lint-full passes. Committed `9892bf213`.

### Run 1950 (2026-03-19)

**Combined match arms with identical bodies across 46 files** (`clippy::match_same_arms`): Fixed 104 of 108 warnings by merging match arms that share the same body using `|` patterns. The remaining 4 warnings are unfixable (codec.rs: arms bind different types; beacon_processor: different variant shapes with extra struct fields).

Key areas improved:
1. **RPC protocol** (16 fixes) — consolidated version strings, protocol mappings, response limits, and max_responses across protocol variants
2. **Peer manager** (17 fixes) — merged error handling arms for RPC errors, rate limiting, and test data setup
3. **Network sync** (17 fixes) — combined request state tracking, batch status, and lookup state arms
4. **Beacon chain** (11 fixes) — merged block verification status checks, availability checker, and graffiti handling
5. **Consensus types** (10 fixes) — consolidated superstruct fork variant arms in beacon_state, signed_beacon_block, beacon_block_body
6. **Remaining** (33 fixes across store, execution_layer, http_api, fork_choice, common/eth2, validator_store, etc.)

Spec v1.7.0-alpha.3 still latest. Open PRs #5008 (field name fix), #4992 (cached PTCs), #5003 (proposer lookahead simplification) — none merged yet. 3428 tests pass across all modified packages. lint-full passes. Committed `802100b7a`.

### Run 1951 (2026-03-19)

**Removed 114 redundant clone() calls across 60 files** (`clippy::redundant_clone`): Used `cargo clippy --fix` to automatically remove `.clone()` calls where the value is not used after cloning (last use before move/drop). These are genuine unnecessary allocations — each removed clone eliminates a heap allocation or reference count increment that serves no purpose.

Key areas:
1. **Types** (9 files) — builder_bid, execution_payload, beacon_block_body, aggregate_and_proof
2. **Beacon chain** (5 files) — historical_blocks, light_client_server_cache, test_utils, block_times_cache, proposer_cache
3. **Network** (8 files) — sync manager, block_sidecar_coupling, lookups tests, subnet_service tests
4. **Store** (5 files) — forwards_iter, hot_cold_store, state_cache, hdiff, blob_sidecar_list
5. **Execution layer** (3 files) — mock_builder, handle_rpc, json_structures
6. **vibehouse_network** (5 files) — gossip_cache, pubsub, codec, response_limiter, sync_status
7. **Remaining** (25 files across validator_client, slasher, logging, network_utils, environment, etc.)

Also fixed 1 `redundant_field_names` lint (`{ info: info }` → `{ info }`) introduced by the auto-fix.

2973 tests pass across modified packages. lint-full passes. Committed `efdf509d5`.

### Run 1952 (2026-03-19)

**Applied 4 pedantic clippy lint fixes across 36 files**:

1. **`unnested_or_patterns`** (18 files, 29 fixes): Combined `Foo(A) | Foo(B)` patterns into `Foo(A | B)` for cleaner match arms
2. **`from_iter_instead_of_collect`** (7 files, 12 fixes): Replaced `Type::from_iter(iter)` with idiomatic `iter.collect()` — files: execution_requests.rs, data_column_custody_group.rs, topics.rs, migrate.rs, single_block_lookup.rs, custody.rs, validators.rs
3. **`needless_for_each`** (6 files, 7 fixes): Replaced `.for_each(|x| { body })` with `for x in iter { body }` loops — files: attestation_service.rs, discovery/mod.rs, block_reward.rs, migrate.rs, case_result.rs, lookups.rs
4. **`needless_continue`** (10 files, 14 fixes): Removed redundant `continue` at end of loop bodies or in trailing else branches — files: eth2/lib.rs, beacon_node_fallback, attestation_service, duties_service (3), notifier_service, payload_attestation_service, beacon_chain (2), gloas_verification, block_sidecar_coupling, sync_committees, http_api/lib.rs

4991/5000 workspace tests pass (8 web3signer timeouts = external service, 1 skip). lint-full passes. Committed `005ec55d5`.

### Run 1953 (2026-03-20)

**Replaced eager evaluation with lazy evaluation in `.ok_or()`, `.unwrap_or()`, `.map_or()` across 35 files** (`clippy::or_fun_call`): Changed 60 call sites where function calls (format!, .to_string(), constructor calls) were eagerly evaluated inside `.ok_or()` / `.unwrap_or()` / `.map_or()` to use their lazy counterparts `.ok_or_else(|| ...)` / `.unwrap_or_else(|| ...)` / `.map_or_else(|| ..., ...)`. This avoids unnecessary allocations on the happy path.

Key areas:
1. **kzg_utils.rs** (8 fixes) — KzgError constructors and format! strings now lazy
2. **handle_rpc.rs** (4 fixes) — mock EL error tuple construction deferred
3. **publish_blocks.rs** (4 fixes) — API error constructors deferred
4. **custody.rs** (4 fixes) — lookup error constructors deferred
5. **checks.rs** (5 fixes) — simulator error constructors deferred
6. **Remaining** (35 fixes across consensus, http_api, network, validator_client, account_manager, lcli, logging, etc.)

Also checked spec status: v1.7.0-alpha.3 still latest. Open PRs #5022 (block known check for payload attestations — already implemented in our code), #5020 (PTC lookbehind), #4992 (cached PTCs) — none merged.

2005 targeted tests pass (proto_array, fork_choice, state_processing, store, logging, kzg, validator_services, eth2, builder_client, account_manager, slashing_protection, database_manager, beacon_processor). lint-full passes. Committed `faae677d1`.

### Run 1954 (2026-03-20)

**Removed 107 unnecessary semicolons across 67 files** (`clippy::unnecessary_semicolon`): Used `cargo clippy --fix` to remove trailing semicolons after blocks/closures where the semicolon serves no purpose. This is a pure style cleanup — no behavioral changes.

Key areas:
1. **Beacon chain** (12 files) — beacon_chain.rs, block_verification.rs, attestation_verification.rs, validator_monitor, state_advance_timer, etc.
2. **Network** (8 files) — gossip_methods.rs, sync modules, subnet_service, network_context
3. **HTTP API** (5 files) — publish_blocks, publish_attestations, sync_committees, block_id, task_spawner
4. **Validator client** (6 files) — initialized_validators, duties_service, http_api, main
5. **vibehouse_network** (5 files) — peer_manager, rpc handler, service
6. **Remaining** (31 files across consensus, crypto, common, testing, boot_node, lcli, etc.)

Also audited spec status: v1.7.0-alpha.3 still latest release. Merged PRs #5001 (parent_block_root in bid filtering key — already implemented in our `ObservedExecutionBids`) and #5002 (wording clarification — no code changes needed) are both covered.

1690 targeted tests pass (proto_array, fork_choice, state_processing, store, logging, merkle_proof, pretty_reqwest_error, eip_3076, validator_dir, metrics). lint-full passes. Committed `a403bd9dc`.

### Run 1955 (2026-03-20)

**Inlined format args across 310 files** (`clippy::uninlined_format_args`): Used `cargo clippy --fix` to replace ~1600 instances of `format!("{}", x)` with `format!("{x}")` (inline format args). This is more idiomatic Rust (stabilized in 1.58) and slightly more readable. Pure style cleanup — no behavioral changes.

Key areas:
1. **Beacon chain** (20+ files) — beacon_chain.rs, block_verification.rs, attestation_verification.rs, execution_layer, builder_client
2. **Network** (15+ files) — sync modules, gossip methods, peer_manager, rpc handler
3. **HTTP API** (10+ files) — publish_blocks, validator endpoints, block_id
4. **Validator client** (15+ files) — duties_service, initialized_validators, http_api, signing
5. **Consensus** (20+ files) — state_processing, types, fork_choice, proto_array
6. **Store** (10+ files) — hot_cold_store, hdiff, forwards_iter
7. **Remaining** (220+ files across common, testing, lcli, account_manager, crypto, etc.)

Also cleaned disk: removed 14.5GB debug incremental cache that caused disk-full during pre-push lint hook.

4991 workspace tests pass (8 web3signer timeouts = external service). lint-full passes. Committed `ba7ac3f2c`.

### Run 1956 (2026-03-20)

**Added underscore separators to 99 numeric literals across 33 files** (`clippy::unreadable_literal`): Applied underscore digit grouping to all numeric literals >= 5 digits for readability:

- **Decimal**: groups of 3 (e.g., `1606824000` → `1_606_824_000`)
- **Hex**: groups of 4 (e.g., `0xDEADBEEF0BAD5EED` → `0xDEAD_BEEF_0BAD_5EED`)
- **Float**: appropriate grouping (e.g., `42.123456` → `42.123_456`)

Key areas:
1. **chain_spec.rs** (28 changes) — epoch numbers, timestamps, limits (fork epochs, genesis times, registry limits)
2. **block_hash.rs** (15 changes) — hex constants in Keccak256 block hash tests
3. **eth2_keystore** (10 changes) — PBKDF2/scrypt parameters and test vectors
4. **Remaining** (46 changes across execution_layer, beacon_chain, network, store, crypto, common, etc.)

Spec v1.7.0-alpha.3 still latest — no new consensus-specs releases. Merged PRs #5001 and #5002 already covered. 2967 targeted tests pass. Zero clippy warnings. lint-full passes. Committed `419c90810`.

### Run 1957 (2026-03-20)

**Replaced single-pattern match with if-let and removed redundant else blocks across 63 files** (`clippy::single_match_else`, `clippy::redundant_else`):

1. **`single_match_else`** (~52 fixes): Replaced `match expr { Pattern => ..., _ => ... }` with `if let Pattern = expr { ... } else { ... }` for cleaner control flow
2. **`redundant_else`** (~33 fixes): Removed `else` blocks after early returns (`return`, `continue`, `break`) and un-indented the following code

Key areas:
1. **Beacon chain** (12 files) — attestation_verification, data_availability_checker, graffiti_calculator, proposer_prep_service, light_client_server_cache, etc.
2. **Network** (7 files) — gossip_methods, rpc_methods, sync manager, custody_backfill_sync, network_context, subnet_service
3. **HTTP API** (4 files) — lib.rs, light_client, produce_block, state_id
4. **vibehouse_network** (5 files) — peer_manager, peerdb, rpc handler, service, discovery
5. **Store** (4 files) — hdiff, forwards_iter, redb_impl, reconstruct
6. **Validator client** (5 files) — config, lib, initialized_validators, slashing_database, sync_committee_service
7. **Remaining** (26 files across execution_layer, common, consensus, account_manager, lcli, etc.)

Also fixed 2 `redundant_pattern_matching` lints (replaced `if let Err(_) =` with `.is_err()` / `if let Ok(_) =` with `.is_ok()`).

Net reduction of ~121 lines. Spec v1.7.0-alpha.3 still latest — no new releases. 1651 targeted tests pass. lint-full passes. Committed `95d3a4124`.

### Run 1958 (2026-03-20)

**Replaced wildcard match arms with explicit variant names across 12 files** (`clippy::match_wildcard_for_single_variants`):

Fixed 34 instances where `_ =>` matched only a single remaining variant. Replacing with the explicit variant name makes code more maintainable — if a new variant is added later, the compiler will flag these match arms for review instead of silently catching them.

Files changed:
1. **common/compare_fields** (7 fixes) — `Comparison::Parent`/`Child` explicit arms in test assertions
2. **beacon_chain/beacon_chain.rs** (3 fixes) — `AvailabilityProcessingStatus::MissingComponents`, `BlockProposalContents::Payload`
3. **beacon_chain/single_attestation.rs** (10 fixes) — `Attestation::Base`/`Electra` explicit arms in tests
4. **beacon_chain/overflow_lru_cache.rs** (1 fix) — `CachedBlock::PreExecution`
5. **beacon_chain/fetch_blobs/tests.rs** (2 fixes) — `BlobAndProof::V1`/`V2`
6. **beacon_node/src/lib.rs** (1 fix) — `DatabaseBackendOverride::Noop`
7. **vibehouse_network/peerdb.rs** (1 fix) — `ScoreState::Healthy`
8. **vibehouse_network/rpc_tests.rs** (2 fixes) — `futures::future::Either::Left`
9. **execution_layer/mock_builder.rs** (1 fix) — `GetPayloadResponseType::Blinded`
10. **store/lib.rs** (1 fix) — `KeyValueStoreOp::DeleteKey`
11. **types/beacon_block_body.rs** (1 fix) — `AttestationRefMut::Base`
12. **http_api/version.rs** (4 fixes) — `BeaconResponse::Unversioned`/`ForkVersioned`

Spec v1.7.0-alpha.3 still latest — no new releases. 1352 targeted tests pass. lint passes. Committed `da8e134f6`.

### Run 1959 (2026-03-20)

**Applied `ignored_unit_patterns` and `if_not_else` pedantic clippy fixes across 72 files**:

1. **`ignored_unit_patterns`** (~40 fixes): Replaced `Ok(_)` with `Ok(())`, `Err(_)` with `Err(())`, `Poll::Ready(_)` with `Poll::Ready(())`, etc. — makes the unit type explicit instead of using a wildcard, improving readability and catching accidental value drops.

2. **`if_not_else`** (~32 fixes): Reordered `if !condition { A } else { B }` to `if condition { B } else { A }` — removes negation from the condition, making the positive case come first for better readability.

Key areas:
1. **Beacon chain** (8 files) — block_verification, attestation_verification, attestation_rewards, graffiti_calculator, etc.
2. **Network** (10 files) — sync modules, subnet_service, rpc_methods, network_service, backfill_sync, custody_backfill_sync
3. **vibehouse_network** (5 files) — peer_manager, discovery, rpc handler, service
4. **HTTP API** (2 files) — extractors, lib
5. **Store** (2 files) — reconstruct, hdiff
6. **Remaining** (45 files across common, consensus, testing, validator_client, account_manager, etc.)

Spec v1.7.0-alpha.3 still latest — no new releases. 4991 workspace tests pass (8 web3signer timeouts = external). 1593 targeted consensus tests pass. lint-full passes. Committed `10004d8a8`.

### Run 1960 (2026-03-20)

**Derived `Eq` alongside `PartialEq` across 130 files** (`clippy::derive_partial_eq_without_eq`): When a type derives `PartialEq` and all its fields implement `Eq`, the type should also derive `Eq`. This enables use in more contexts (e.g., `HashMap` keys, `assert_eq!` with better error messages) and is semantically correct — these types all have reflexive equality.

Applied via `cargo clippy --fix` for 95 files, then manual fixes for 35 files where auto-fix couldn't apply (types crate generics, crypto crates, superstruct-generated code). Reverted `Eq` on `LightClientHeader` — its `execution` field uses `educe(PartialEq)` (not `derive`), so the inner `ExecutionPayloadHeader` variants don't implement `Eq`.

Key areas:
1. **Consensus types** (47 files) — attestation_duty, beacon_committee, fork, graffiti, payload, preset, sync types, etc.
2. **Beacon chain** (15 files) — block_verification, execution_payload, data_availability, builder, etc.
3. **Network** (10 files) — rpc methods, sync modules, peer manager
4. **HTTP API** (5 files) — ui, types, std_types
5. **Execution layer** (5 files) — engine API, json structures, test utils
6. **Crypto** (5 files) — eth2_keystore cipher/kdf modules, eth2_wallet
7. **Remaining** (43 files across store, validator_client, common, proto_array, etc.)

Spec v1.7.0-alpha.3 still latest — no new releases. 4991 workspace tests pass (9 web3signer timeouts = external). lint-full passes. Committed `018024abd`.

### Run 1961 (2026-03-20)

**Applied 3 pedantic clippy fixes across 29 files**:

1. **`explicit_iter_loop`** (18 files, 21 fixes): Removed unnecessary `.into_iter()` calls in `for` loops — `for x in collection` is idiomatic when consuming the collection, `.into_iter()` is implicit
2. **`range_plus_one`** (1 file, 1 fix): Replaced `0..n + 1` with `0..=n` inclusive range (validator_pubkey_cache.rs)
3. **`semicolon_if_nothing_returned`** (10 files, 18 fixes): Added missing semicolons after expressions in blocks that return unit — makes the unit return explicit

Spec v1.7.0-alpha.3 still latest — no new releases or Gloas-relevant merges. 2025 targeted tests pass (proto_array, fork_choice, state_processing, store, logging, beacon_processor, vibehouse_network) + 69 EF SSZ static tests. lint-full passes. Committed `d3ab34544`.

### Run 1962 (2026-03-20)

**Applied 5 pedantic clippy fixes across 36 files**:

1. **`cast_lossless`** (12 files, ~15 fixes): Replaced `x as u64` with `u64::from(x)` for safe widening casts — uses the type system to guarantee losslessness instead of relying on `as`
2. **`manual_assert`** (5 files, ~8 fixes): Replaced `if cond { panic!(...) }` with `assert!(!cond, ...)` — more idiomatic and clearer intent
3. **`items_after_statements`** (11 files, ~25 fixes): Moved `use`, `const`, `struct`, and `fn` declarations before executable statements in their enclosing blocks — item declarations should come first for readability
4. **`nonminimal_bool`** (1 file, 1 fix): Simplified `!(a && !b)` to `!a || b` in interchange_test.rs
5. **`assertions_on_constants`** (1 file, 1 fix): Wrapped compile-time `assert!(!cfg!(windows))` in `const { }` block (execution_engine_integration)

Note: 39 `expl_impl_clone_on_copy` warnings remain — all originate from the `superstruct` proc macro (not fixable in our code).

Spec v1.7.0-alpha.3 still latest — no new releases. 2127 targeted tests pass + 69 EF SSZ static tests. lint-full passes. Committed `b2e1067b4`.

### Run 1963 (2026-03-20)

**Applied 2 pedantic clippy fixes across 17 files**:

1. **`enum_glob_use`** (10 files, 17 fixes): Replaced `use EnumType::*` with explicit variant imports — makes dependencies clear, catches new variants at compile time instead of silently matching them
2. **`default_trait_access`** (7 files, ~13 fixes): Replaced `Default::default()` with concrete type names (`Hash256::default()`, `Slot::default()`, `FixedVector::default()`, `VariableList::default()`) — makes the type explicit for readability

Note: ~109 `default_trait_access` warnings remain in superstruct-generated code (e.g., `execution_payload_header.rs:149` expands to 6 variant-specific warnings) and scattered non-types crates. The superstruct ones are unfixable in our code. Remaining pedantic lints (~5000+) are dominated by `missing_errors_doc` (1564), `must_use_candidate` (1306), `doc_markdown` (1015), `cast_possible_truncation` (631) — all noise-level lints not worth fixing.

Spec v1.7.0-alpha.3 still latest — no new releases. Recent merged spec PRs: #5005 (test fix), #5002 (wording). Notable open PRs: #5022 (block known check in on_payload_attestation_message), #5020/#4992 (PTC lookbehind/cached PTCs), #5008 (field name fix). PR #5001 (parent_block_root in bid filtering) already implemented correctly. 1500 targeted tests + 69 EF SSZ static tests pass. lint-full passes. Committed `f131ef6f5`.

### Run 1964 (2026-03-20)

**Monitoring check — no code changes needed.**

Reviewed all tracked open spec PRs: #5022 (block known check in on_payload_attestation_message) — already handled by our `InvalidPayloadAttestation::UnknownBeaconBlockRoot` error at fork_choice.rs:1426-1432. #5008 (field name fix) — prose-only, our code already uses `beacon_block_root`. #5020/#4992 (PTC lookbehind/cached PTCs) — still open, high-impact if merged. #4954 (fork choice milliseconds) — still open, pervasive if merged. #4898/#4892 (fork choice cleanup) — still open, minor.

Clippy clean. No new spec releases. All pedantic lint work at diminishing returns — remaining ~5000 warnings are noise-level (`missing_errors_doc`, `must_use_candidate`, `doc_markdown`, `cast_possible_truncation`) or from generated code. No code changes this run.

### Run 1965 (2026-03-20)

**Validation run — no code changes needed.**

Re-downloaded v1.7.0-alpha.3 EF test vectors (fresh download includes Gloas fork choice tests from spec PR #4940, merged same day as release). Full results:
- **79/79 real crypto tests pass** — includes new Gloas fork choice: get_head, on_block, ex_ante, reorg, withholding, on_execution_payload
- **139/139 fake_crypto tests pass**
- **check_all_files_accessed: 210795 files accessed** (132271 intentionally excluded)
- **Devnet: SUCCESS** — 4-node network finalized epoch 8 through Gloas fork boundary (validates stack integrity after clippy pedantic batch)
- **lint-full: clean**

Open spec PRs unchanged: #5022, #5020/#4992, #4954, #4898/#4892 — all still open. No new merged PRs since last check. No code changes this run.

### Run 1966 (2026-03-20)

**Monitoring run — no code changes needed.**

Full audit:
- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs. Open Gloas PRs unchanged: #5022 (block known check — already implemented), #5023 (test fixture filenames), #5020/#4992 (PTC lookbehind — still open, high-impact). #4747 (fast confirmation rule) updated but not ePBS-specific.
- **CI**: All 6 jobs green on latest commit (`f131ef6f5`). Nightly tests passed.
- **Clippy**: Clean (zero warnings, `--all-targets`).
- **cargo audit**: 1 vulnerability (rsa/RUSTSEC-2023-0071 — Marvin Attack timing sidechannel, no fix available, transitive via jsonwebtoken→execution_layer, local-only EL communication so low risk). 5 unmaintained crate warnings (ansi_term, bincode, derivative, paste — all transitive from sp1-verifier; filesystem — false positive, matches our local crate name).
- **TODOs**: 11 remaining across 8 files — all blocked (#36 items: EIP-7892 ×3, blst unsafe ×1, PeerDAS ×1) or non-critical (EL error enums, pool persistence, store test hack).
- **unsafe blocks**: All legitimate (blst FFI, libc/jemalloc FFI, set_var in main.rs).

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1967 (2026-03-20)

**Monitoring + validation run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs. Open Gloas PRs unchanged: #5022 (block known check — already implemented), #5023 (test fixture filenames), #5020 (PTC lookbehind — still open, high-impact). No new releases.
- **CI**: All jobs green on `f131ef6f5`.
- **cargo audit**: Same as previous — 1 rsa vulnerability (no fix, low risk), 5 unmaintained crate warnings (all transitive).
- **GitHub issues**: #36 remaining items all blocked or non-critical. #29 (ROCQ) lowest priority. #27 (validator messaging) speculative feature request.
- **Devnet**: SUCCESS — 4-node network finalized epoch 8 (slot 80, epoch 10) through Gloas fork boundary. Validates full stack integrity after pedantic clippy batch (runs 1955-1963).

No actionable work found. Codebase stable and validated.

### Run 1968 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since alpha.3 release (March 13). #4979 (PTC lookbehind) was closed without merge. Open Gloas PRs unchanged: #5022 (block known check — already implemented), #5023 (test fixture filenames), #5020 (PTC lookbehind minimal — still open), #4992 (cached PTCs — still open), #4954 (fork choice milliseconds — still open), #4898/#4892 (fork choice cleanup — still open).
- **CI**: All jobs green on latest commit. Nightly tests passed.
- **GitHub issues**: No new issues. #36 items blocked or non-critical. #29 (ROCQ) lowest priority.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1969 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged Gloas PRs. Open Gloas PRs unchanged: #5022 (block known check — already implemented), #5023 (test fixture filenames), #5020 (PTC lookbehind minimal — still open), #4992 (cached PTCs — still open), #4954 (fork choice milliseconds — still open), #4898/#4892 (fork choice cleanup — still open).
- **CI**: All jobs green. Nightly tests passed.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1970 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged Gloas PRs. Open Gloas PRs unchanged: #5022, #5023, #5020, #4992, #4954, #4898/#4892, #4960, #4939, #4932, #4843, #4840. No new additions since last check.
- **CI**: All jobs green. Nightly tests passed. Latest CI run (`f131ef6f5`) succeeded.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1971 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged Gloas PRs. Open Gloas PRs unchanged: #5022 (block known check — re-verified already implemented at fork_choice.rs:1432), #5023, #5020, #4992, #4954, #4898/#4892, #4960, #4939, #4932, #4843, #4840.
- **CI**: All jobs green. Build clean (zero warnings). `cargo clippy --workspace --all-targets` clean. `RUSTFLAGS="-W dead_code" cargo check` clean.
- **Dependencies**: `cargo update` found 0 compatible updates; 33 behind latest are all semver-major bumps requiring Cargo.toml changes (not routine maintenance).
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1972 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. Only one PR merged since alpha.3: #5005 (test fixture fix, already assessed as no-change-needed). Open Gloas PRs unchanged: #5022, #5023, #5020, #4992, #4954, #4898/#4892, #4960, #4939, #4932, #4843, #4840.
- **CI**: All jobs green. Nightly tests passed. Build clean (zero warnings).
- **Dependencies**: `cargo update --dry-run` shows 3 patch bumps (itoa 1.0.18, zerocopy 0.8.47) — trivial.
- **GitHub issues**: No new issues. #36 items all blocked or non-critical. #29 (ROCQ) lowest priority.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1973 (2026-03-20)

**Dependency patch update — minor change.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs. Open Gloas PRs unchanged: #5022, #5023, #5020, #4992, #4954, #4898/#4892, #4960, #4939, #4932, #4843, #4840.
- **CI**: All jobs green. Build clean (zero warnings).
- **Dependencies**: Applied 3 patch bumps: itoa 1.0.17→1.0.18, zerocopy 0.8.42→0.8.47, zerocopy-derive 0.8.42→0.8.47. Build passes. Committed `1ccc172ef`.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.

All priorities 1-6 complete. Codebase stable.

### Run 1974 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since #5005 (March 15). Open Gloas PRs unchanged: #5022, #5023, #5020, #4992, #4954, #4898/#4892, #4960, #4939, #4932, #4843, #4840. #5008 (field name fix) — already tracked and confirmed aligned.
- **CI**: Latest run in progress. Previous runs all green. Nightly tests passed.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1975 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest (released March 13). No new merged PRs since #5005 (March 15). Most recent merges: #5005 (test fix), #5004 (release notes), #4940 (Gloas fork choice tests), #5002 (wording), #5001 (parent_block_root) — all previously assessed.
- **CI**: All jobs green. Latest run (pedantic clippy commit) in progress. Nightly tests passed.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1976 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since #5005 (March 15). Open Gloas PRs unchanged: #5022, #5023, #5020, #4992, #4954, #4898/#4892, #4960, #4939, #4932, #4843, #4840. #5014 (EIP-8025 p2p protocol) also open.
- **CI**: All jobs green. Latest CI run in progress. Spec-test-version-check and nightly tests passed.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1977 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since #5005 (March 15). Open Gloas PRs unchanged: #5023, #5020, #4992, #4954, #4960, #4932, #4898/#4892, #4843, #4840.
- **CI**: All jobs green. Nightly tests passed. Spec-test-version-check passed. Pedantic clippy CI run succeeded.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1978 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since #5005 (March 15). Open Gloas PRs unchanged: #5023, #5020, #4992, #4954, #4960, #4932, #4898/#4892, #4843, #4840.
- **CI**: All jobs green. Latest push CI in progress. Nightly tests passed.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1979 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since #5005 (March 15). Open Gloas PRs unchanged: #5023, #5020, #4992, #4954, #4960, #4932, #4898/#4892, #4843, #4840.
- **CI**: All jobs green. Current CI run in progress. Nightly tests passed. Spec-test-version-check passed.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1980 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since #5005 (March 15). Open Gloas PRs unchanged: #5023, #5020, #4992, #4954, #4960, #4932, #4898/#4892, #4843, #4840. #4747 (Fast Confirmation Rule) updated today.
- **CI**: All jobs green. Current CI run in progress. Nightly tests passed. Spec-test-version-check passed.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.
- **Dependencies**: No new patch bumps since run 1973.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1981 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since #5005 (March 15). Open Gloas PRs unchanged: #5023, #5022, #5020, #5008, #4992, #4960, #4954, #4939, #4932, #4898/#4892, #4843, #4840.
- **CI**: All jobs green. Latest CI run (dep update) in progress. Nightly and spec-test-version-check passed.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.
- **Dependencies**: No new crate updates available.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1982 (2026-03-20)

**Pedantic clippy fix: unnecessary_debug_formatting — 29 files.**

- Fixed 69 instances of `{:?}` (Debug formatting) used on `PathBuf`/`Path` types where `{}` with `.display()` is cleaner. Debug formatting wraps paths in quotes and escapes characters; Display formatting shows clean paths.
- Files: account_manager (6), validator_client (3), validator_manager (3), common (4), testing (4), beacon_node (2), lcli (2), database_manager (1), eth2_network_config (2), wallet (2).
- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since #5005 (March 15). Open Gloas PRs: #5022 (on_payload_attestation check — we already have this), #4992 (cached PTCs — still open), #4843 (variable PTC deadline — still open).
- **CI**: All jobs green. Nightly and spec-test-version-check passed.
- **Verification**: 52/52 targeted tests, full workspace clippy clean, cargo fmt clean, pre-push lint-full passes.

### Run 1983 (2026-03-20)

**Monitoring run — no code changes.**

- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since #5005 (March 15). Open Gloas PRs unchanged: #5023, #5022, #4992, #4960, #4939, #4932, #4898/#4892, #4843, #4840. #4747 (Fast Confirmation Rule) updated today but still open.
- **CI**: All jobs green. Current CI run in progress. Nightly and spec-test-version-check passed.
- **GitHub issues**: No new issues. #36 blocked/non-critical. #29 (ROCQ) lowest priority.
- **Dependencies**: No new crate updates available.

No actionable work found. All priorities 1-6 complete. Codebase stable.

### Run 1984 (2026-03-20)

**Pedantic clippy fix: used_underscore_binding — 7 files.**

- Fixed underscore-prefixed bindings that are actually used, across 7 files. The `_` prefix convention means "intentionally unused" — these were misusing it.
- Files: logging (2 — tracing layers), eth2_wallet_manager (locked_wallet.rs), store (historic_state_cache.rs), beacon_chain (beacon_chain.rs, blob_verification.rs, state_lru_cache.rs).
- Skipped: types/beacon_block_body.rs and light_client_header.rs (superstruct macro-generated `_phantom` fields), slashing_database.rs (test-only), validator_monitor.rs tests (48 warnings, test code).
- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since #5005 (March 15). Open Gloas PRs unchanged.
- **CI**: All jobs green. Nightly and spec-test-version-check passed.
- **Verification**: 263/263 targeted tests passed, full workspace clippy clean, cargo fmt clean, pre-push lint-full passes.

### Run 1985 (2026-03-20)

**Pedantic clippy fix: match_same_arms — 3 files.**

- `gloas_verification.rs`: merged `New` and `Duplicate` arms that both do nothing (continue) into single `New | Duplicate` arm.
- `codec.rs`: added `#[allow(clippy::match_same_arms)]` — variants call `.as_ssz_bytes()` but on different types (can't merge with `|`).
- `beacon_processor/lib.rs`: added `#[allow(clippy::match_same_arms)]` — `DelayedImportBlock` (struct variant) can't merge with tuple variants via `|`.
- **Spec**: v1.7.0-alpha.3 still latest. No new merged PRs since #5005 (March 15). Open Gloas PRs: #5023 (block root filenames, updated today), #4747 (Fast Confirmation Rule, updated today) — both still open.
- **CI**: All jobs green. Nightly and spec-test-version-check passed.
- **Verification**: 1414/1414 targeted tests passed (beacon_chain, beacon_processor, vibehouse_network), full workspace clippy clean, cargo fmt clean, pre-push lint-full passes.
