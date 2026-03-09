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

### 2026-03-09 — spec scan (run 647)
- All 12 tracked Gloas PRs still OPEN, no new merges, no new PRs
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI: 4/6 passed, 2 in progress; cargo audit/outdated clean (1 known rsa advisory)
- No code changes — spec stable, fully compliant

### 2026-03-09 — spec scan (run 646)
- All 12 tracked Gloas PRs still OPEN, no new merges, no new PRs
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- CI: 4/6 passed, 2 in progress; cargo audit unchanged
- No code changes — spec stable, fully compliant

### 2026-03-09 — spec scan (run 645)
- All 12 tracked Gloas PRs still OPEN, no new merges, no new PRs
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI: 4/6 passed, 2 still running; nightly all 27 jobs green
- No code changes — spec stable, fully compliant

### 2026-03-09 — spec scan (run 644)
- All 12 tracked Gloas PRs still OPEN, no new merges
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI green, nightly 3x consecutive green (Mar 7-9)
- No code changes — spec stable, fully compliant

### 2026-03-09 — spec scan (run 643)
- Now tracking 12 Gloas PRs (added #4939 index-1 attestation envelope validation, #4962 missed payload withdrawal tests)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- CI green, no code changes — spec stable, fully compliant

### 2026-03-09 — spec scan (run 642)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- CI green, nightly consecutive green since Mar 4
- No code changes — spec stable, fully compliant

### 2026-03-09 — consolidated: runs 524-641 (Mar 7-9)
Key activities across ~120 runs (routine scans omitted):
- **run 641**: docker CI paths-ignore for docs-only commits
- **run 640**: post-rebrand devnet verification SUCCESS
- **run 578**: upgraded ethabi 16→18
- **run 577**: upgraded 7 dependencies (jsonwebtoken 9→10, rpassword 5→7, etc.)
- **run 576**: upgraded RustCrypto suite + sha2
- **run 575**: upgraded redb 2.x→3.1.0
- **run 574**: replaced psutil with procfs
- **run 572**: switched default DB to redb
- **run 555**: deep spec conformance audit — all checks verified correct
- **run 547**: fixed gossip message leak
- **run 545**: automated spec release check workflow, CI concurrency fix
- **run 521**: fixed flaky CI test
- **runs 516-523**: test coverage improvements (HTTP API, gossip, fork choice, error paths)
- Throughout: continuous spec monitoring, zero new spec releases, all 10 tracked PRs remain open

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
