# Code Review & Quality Improvement

## Goal
The loop has shipped a massive amount of code autonomously (100+ runs). Review the codebase for quality issues, technical debt, and correctness concerns that may have accumulated during rapid autonomous development.

## Strategy

### Phase 1: Audit & Inventory
- [ ] Run `cargo clippy --workspace -- -W clippy::all` and fix all warnings
- [ ] Run `cargo doc --workspace --no-deps` — fix any doc warnings
- [ ] Identify dead code, unused imports, unreachable paths
- [x] Check for `unwrap()`/`expect()` in non-test code — replace with proper error handling
- [x] Look for `todo!()`, `unimplemented!()`, `fixme`, `hack` comments

### Phase 2: Architecture Review
- [ ] Review public API surface — are things `pub` that shouldn't be?
- [ ] Check module organization — any god-files that should be split?
- [ ] Review error types — consistent error hierarchy? Good error messages?
- [ ] Check for code duplication across Gloas fork variants
- [ ] Review superstruct variant handling — any missing arms, fallthrough bugs?

### Phase 3: Correctness Deep-Dive
- [ ] Cross-reference Gloas implementation against consensus-specs v1.7.0-alpha.2
- [ ] Verify all spec constants match (domain types, config values, timing)
- [ ] Review edge cases in state transitions — overflow, underflow, empty collections
- [ ] Audit builder payment/withdrawal logic for economic bugs
- [ ] Review fork choice weight calculations against spec

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
