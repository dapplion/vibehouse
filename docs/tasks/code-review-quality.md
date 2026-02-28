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

### Phase 2: Architecture Review
- [ ] Review public API surface — are things `pub` that shouldn't be?
- [ ] Check module organization — any god-files that should be split?
- [ ] Review error types — consistent error hierarchy? Good error messages?
- [ ] Check for code duplication across Gloas fork variants
- [ ] Review superstruct variant handling — any missing arms, fallthrough bugs?

### Phase 3: Correctness Deep-Dive
- [x] Cross-reference Gloas implementation against consensus-specs v1.7.0-alpha.2
- [ ] Verify all spec constants match (domain types, config values, timing)
- [ ] Review edge cases in state transitions — overflow, underflow, empty collections
- [x] Audit builder payment/withdrawal logic for economic bugs
- [x] Review fork choice weight calculations against spec

### Phase 4: Performance
- [ ] Profile hot paths (state transition, block processing, attestation validation)
- [ ] Check for unnecessary clones, allocations in tight loops
- [ ] Review database access patterns — any N+1 queries?
- [ ] Check serialization/deserialization efficiency

### Phase 5: Test Quality
- [ ] Review test coverage gaps — which critical paths lack tests?
- [ ] Check test assertions — are they testing the right things?
- [ ] Look for flaky/non-deterministic tests
- [ ] Ensure integration tests cover realistic scenarios

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
