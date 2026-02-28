# Code Review & Quality Improvement

## Goal
The loop has shipped a massive amount of code autonomously (100+ runs). Review the codebase for quality issues, technical debt, and correctness concerns that may have accumulated during rapid autonomous development.

## Strategy

### Phase 1: Audit & Inventory
- [ ] Run `cargo clippy --workspace -- -W clippy::all` and fix all warnings
- [ ] Run `cargo doc --workspace --no-deps` — fix any doc warnings
- [ ] Identify dead code, unused imports, unreachable paths
- [ ] Check for `unwrap()`/`expect()` in non-test code — replace with proper error handling
- [ ] Look for `todo!()`, `unimplemented!()`, `fixme`, `hack` comments

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
<!-- Append findings as they're discovered -->
