# Spec Tests

## Objective
Run the latest consensus spec tests at all times. Track and fix failures.

## Status: DONE

### Current results
- **78/78 ef_tests pass (real crypto, 0 skipped)** — both mainnet + minimal presets
- **138/138 fake_crypto pass (0 skipped)** — both mainnet + minimal presets (Fulu + Gloas DataColumnSidecar variants both pass)
- **check_all_files_accessed passes** — 209,677 files accessed, 122,748 intentionally excluded
- All 8 fork_choice test categories pass (get_head, on_block, ex_ante, reorg, withholding, get_proposer_head, deposit_with_reorg, should_override_forkchoice_update)
- 40/40 gloas execution_payload envelope tests pass (process_execution_payload_envelope spec validation)
- rewards/inactivity_scores tests running across all forks (was missing)
- 3 altair proposer_boost tests now pass (were skipped, sigp/lighthouse#8689 — fixed by implementing PR #4807)

### Tasks
- [x] Audit spec test runner — understand download, cache, run flow
- [x] Check which spec test version is currently pinned (v1.7.0-alpha.2)
- [x] Update to latest spec test release when new ones drop
- [x] Ensure all existing fork tests pass (phase0 through fulu)
- [x] Add gloas test scaffolding: register fork, add handlers, wire new test types
- [x] Set up CI job: download latest vectors, run all tests, fail on regression
- [x] Create automated check for new spec test releases

### Test categories
bls, epoch_processing, finality, fork, fork_choice, genesis, light_client, operations, random, rewards, sanity, ssz_static, transition

## Progress log

### runs 959-1144 consolidated (Mar 11-13) — spec stable, monitoring only
Spec completely stable since v1.7.0-alpha.3 version bump (#4999, Mar 11). No new spec-test vectors (still v1.6.0-beta.0). No new formal release (still v1.7.0-alpha.2 published). All tracked spec-test PRs (#4932, #4939, #4960, #4962) remain OPEN. PR #4992 (PTC lookbehind): OPEN, MERGEABLE, 1 APPROVED (jtraglia), same head d76a278b0a, new comment from jihoonsong (Mar 13). CI and nightly continuously green. cargo audit unchanged (1 rsa). PR #4940 (Gloas fork choice tests) merged Mar 13 — test generators only, no new vectors yet. ptc-lookbehind branch rebased onto main, 575/575 state_processing tests pass. PR #5001 (parent_block_root in bid filtering key) merged — already implemented in vibehouse. PR #5002 (wording clarification) — no code change needed. PR #5004 (release notes dependencies section) — tooling only. No semver-compatible dep updates. Codebase audit: no TODOs/FIXMEs/untested paths in Gloas code.

Notable activities:
- Run 1054: Committed Cargo.lock transitive dep update (windows-sys 0.61.2, syn 2).
- Run 988: Added 5 SSZ round-trip tests for proto_array Gloas fields (ProtoNode, VoteTracker, SszContainer).
- Runs 994, 1005, 1008, 1015, 1031: Multiple code/test coverage audits — all Gloas consensus paths verified correct, no unwraps, all safe arithmetic, comprehensive integration test coverage.
- Run 959: Verified all 7 alpha.3 changes already implemented.

### runs 759-958 consolidated (Mar 10-13) — spec stable, PTC lookbehind implemented
Spec completely stable — no new consensus-specs commits with consensus changes since #5001 (Mar 12), no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). All 11 tracked Gloas PRs remained OPEN throughout. PR #4992 (PTC lookbehind) evolved from head 215962a9 (blocked) to d76a278b0a (clean, 1 APPROVED jtraglia Mar 12). CI and nightly continuously green. EF spec tests: consistently 35/35 (minimal, fake_crypto), fork choice: 8/8 (real crypto). Workspace tests: 2643/2651 (8 web3signer timeout). cargo audit: 1 rsa advisory (no fix). Recent consensus-specs merges were all CI/tooling (#4984 remove Verkle, #4988-#4995 Python/reftest/release-drafter).

Notable activities:
- Run 929: Implemented PTC lookbehind on branch `ptc-lookbehind` (previous_ptc/current_ptc fields, compute_ptc, get_ptc cached reads, per_slot rotation, upgrade initialization). All 575 state_processing tests pass. NOT merged — blocks on PR #4992 merge + new spec test vectors.
- Run 929: Fixed `clippy::large_stack_frames` in `proposer_boost_re_org_test` (Rust 1.91 bump)
- Run 926: Updated deps (clap 4.6, openssl 0.10.76, c-kzg 2.1.7, tempfile 3.27)
- Run 871: Updated Cargo.lock (windows-sys transitive deps)
- Run 850: Added workflow_dispatch trigger to ci.yml
- Run 834: Codebase audit — 39 TODOs (all inherited/spec-dependent), gloas.rs has 208 unit tests across 9216 lines
- Run 800: Analyzed PTC lookbehind implementation plan (7 code areas)

### 2026-03-09 — consolidated: runs 524-758 (Mar 7-10)
Key activities across ~230 runs:
- **run 735**: Fixed 2 beacon_chain test failures (slasher backend guard, Fulu fork scheduling check)
- **run 723-725**: Added 22 proto_array tests (propagation, validation, invalidation, viability, contains_invalid_payloads, on_invalid_execution_payload)
- **run 718**: Deep spec conformance audit — all Gloas functions verified correct against consensus-specs master
- **run 717**: Added 6 tests for `process_payload_attestation` + `get_indexed_payload_attestation`
- **run 701**: Implemented PR #4939 (index-1 attestation envelope validation) proactively
- **run 680-677**: Updated zerocopy, alloy-trie, quinn-proto, yamux deps
- **run 676**: Added 4 prometheus gauge metrics for ePBS pool monitoring
- **run 675**: Added 5 epoch processing integration/edge case tests
- **run 641**: docker CI paths-ignore for docs-only commits
- **run 640**: post-rebrand devnet verification SUCCESS
- **run 578**: upgraded ethabi 16→18
- **run 577**: upgraded 7 dependencies (jsonwebtoken 9→10, rpassword 5→7, etc.)
- **run 572-576**: switched default DB to redb, upgraded RustCrypto suite, replaced psutil with procfs
- **run 547**: fixed gossip message leak
- **run 545**: automated spec release check workflow, CI concurrency fix

### 2026-03-07 — consolidated: runs 37-523 (Feb 20 - Mar 7)
~480 runs of test writing, spec monitoring, and maintenance. Key milestones:
- **Feb 20-Mar 1**: wrote 800+ unit tests across all Gloas subsystems (fork choice, state processing, gossip verification, beacon chain, HTTP API, types, validator client)
- **Mar 1-3**: external builder integration tests, devnet test scenarios (sync, churn, mainnet, long-running, builder, partition, slashing)
- **Mar 3-5**: code review & quality improvement (5 phases: clippy/doc audit, architecture review, correctness deep-dive, performance audit, test quality)
- **Mar 5-7**: dependency upgrades, redb migration, CI improvements

### 2026-02-19 — full-preset EF test verification (mainnet + minimal)
- Both presets pass: 78/78 real crypto, 138/138 fake_crypto

### 2026-02-18 — fix fork_choice_on_block for Gloas blocks (77/78 → 78/78)
- Fixed Gloas block on_block handler to properly set bid fields

### 2026-02-19 — add ProposerPreferences SSZ types (136→138 fake_crypto tests)
- Added SSZ serialization for ProposerPreferences, fixing 2 remaining test failures

### 2026-02-17 — fix check_all_files_accessed (was failing with 66,302 missed files)
- Registered all Gloas test directories in the test runner

### 2026-02-17 — 78/78 passing (execution_payload envelope tests added)
- Added envelope test handlers, all passing

### 2026-02-17 — 77/77 passing (DataColumnSidecar SSZ fixed)
- Fixed Gloas variant for DataColumnSidecar serialization

### 2026-02-15 — 76/77 passing
- Initial Gloas test scaffolding complete

### 2026-02-14 — SSZ static pass
- First pass at Gloas SSZ static tests
