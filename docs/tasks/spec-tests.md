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

### 2026-03-09 — new PTC Lookbehind alternative PR (run 683)
- Spec stable: no new merges, no new releases
- New PR #4992 "Add PTC Lookbehind" (potuz, Mar 9): simpler alternative to #4979, only 2 entries (~8KB) instead of 2*SLOTS_PER_EPOCH (~256KB), updated per-slot in process_slots
- CI run 680: check+clippy ✓, ef-tests ✓, network+op_pool ✓, 3 jobs still running
- No code changes — waiting for one of #4979/#4992 to merge

### 2026-03-09 — full scan, all green (run 682)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- PTC Lookbehind (#4979) still blocked (mergeable but needs approvals, 10 review comments, last activity Mar 7)
- New PRs tracked: #4747 (Fast Confirmation Rule, FOCIL/7805, not core ePBS), #4558 (Cell Dissemination), #4954 (fork choice milliseconds)
- CI run 680: check+clippy ✓, ef-tests ✓, remaining jobs in progress; nightly 5 consecutive greens (Mar 5-9)
- Zero compilation warnings, zero clippy warnings, cargo audit unchanged (1 medium rsa advisory, transitive)
- All patch deps at latest versions, no semver-compatible updates available
- Codebase audit: no new untested critical error paths; consensus code (state_processing, fork_choice, proto_array) has zero production unwrap() calls
- No code changes needed — spec stable, codebase in excellent shape

### 2026-03-09 — spec scan + PTC Lookbehind analysis (run 681)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- 7 tracked Gloas PRs still OPEN (#4979, #4960, #4940, #4932, #4843, #4840, #4630); #4962 and #4939 no longer appear in filtered results
- PTC Lookbehind (#4979) analysis: adds `ptc_lookbehind: Vector[Vector[ValidatorIndex, PTC_SIZE], 2 * SLOTS_PER_EPOCH]` to BeaconState, new `compute_ptc`/`process_ptc_lookbehind` functions, modifies `get_ptc` to use cache. Still under review (10 comments). When merged: need new state field, epoch processing step, fork upgrade changes, ~medium scope
- PRs #4940/#4960 add Gloas fork choice test vectors — already handled by existing `ForkChoiceHandler` which iterates all forks
- PR #4932 adds Gloas sanity/blocks tests — already handled by existing `SanityBlocksHandler`
- CI run 680 in progress (all 6 jobs running), nightly 5 consecutive greens (Mar 5-9)
- Clippy clean (zero warnings), cargo audit unchanged (1 medium rsa advisory, transitive)
- No code changes needed — spec stable, test infrastructure ready for incoming test vectors

### 2026-03-09 — dep update + spec scan (run 680)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 9 tracked Gloas PRs still OPEN (#4979, #4962, #4960, #4940, #4939, #4932, #4843, #4840, #4630)
- CI green (check+clippy passed, 5 jobs in progress), nightly 5 consecutive greens (Mar 5-9)
- Updated patch dep: zerocopy 0.8.41→0.8.42
- All 2612 workspace tests pass (excl. web3signer infra-dependent), 1282 types+state_processing pass
- cargo audit unchanged (1 medium rsa advisory, transitive)

### 2026-03-09 — spec scan (run 679)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 9 tracked Gloas PRs still OPEN (#4979, #4962, #4960, #4940, #4939, #4932, #4843, #4840, #4630)
- PTC Lookbehind (#4979) still blocked (last activity Mar 7)
- Fast Confirmation Rule (#4747) updated today — EIP-7805/FOCIL feature, not core ePBS, no action needed
- CI in progress for run 678 push, nightly 3 consecutive greens (Mar 7-9)
- Clippy clean (zero warnings), cargo audit unchanged (1 medium rsa advisory, transitive)
- No code changes needed — spec stable, codebase in excellent shape

### 2026-03-09 — PTC edge case test + spec scan (run 678)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 9 tracked Gloas PRs still OPEN (#4979, #4962, #4960, #4940, #4939, #4932, #4843, #4840, #4630)
- PTC Lookbehind (#4979) still blocked (last activity Mar 7)
- CI in progress (all 6 jobs running), clippy clean, cargo audit unchanged (1 medium rsa advisory, transitive)
- Added `ptc_committee_no_validators_returns_error` test: verifies `get_ptc_committee` returns `NoActiveValidators` error when committee has zero validators (previously untested error path)
- Audited existing test coverage: bid processing error paths (out-of-bounds, insufficient balance, pubkey decompression, invalid signature, inactive builder) all already thoroughly tested

### 2026-03-09 — dep updates + spec scan (run 677)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 9 tracked Gloas PRs still OPEN (#4979, #4962, #4960, #4940, #4939, #4932, #4843, #4840, #4630)
- PTC Lookbehind (#4979) still blocked (mergeable but no approvals, last activity Mar 7)
- CI green, clippy clean, cargo audit unchanged (1 medium rsa advisory, transitive)
- Updated patch deps: alloy-trie 0.9.4→0.9.5, quinn-proto 0.11.13→0.11.14, yamux 0.13.9→0.13.10
- All 2611 workspace tests pass (excl. web3signer infra-dependent), 163 network tests pass, 46 execution_layer tests pass

### 2026-03-09 — prometheus metrics for ePBS pools + spec scan (run 676)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 9 tracked Gloas PRs still OPEN (#4979, #4962, #4960, #4940, #4939, #4932, #4843, #4840, #4630)
- CI green (nightly 5+ consecutive greens), clippy clean, cargo audit unchanged
- Added 4 prometheus gauge metrics for ePBS pool monitoring:
  - `beacon_execution_bid_pool_size`: bids in the bid pool
  - `beacon_observed_payload_attestations_size`: tracked payload attestations for dedup
  - `beacon_observed_payload_envelopes_size`: tracked envelope roots for dedup
  - `beacon_observed_execution_bids_size`: tracked bid dedup entries
- Made `total_bid_count`/`bid_count_for_slot`/`slot_count` public on ExecutionBidPool (were #[cfg(test)])
- All 401 Gloas beacon_chain tests pass, 49 execution bid tests pass

### 2026-03-09 — epoch processing tests + spec scan (run 675)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 9 tracked Gloas PRs still OPEN
- Added 5 epoch processing integration/edge case tests:
  - Fulu fork gate skips builder payments even when config enabled
  - Gloas proposer lookahead entries match independent computation
  - Combined payments + lookahead no interference
  - Large total_active_balance quorum arithmetic (safe_arith)
  - Withdrawals preserved after effective balance updates
- All 566 state_processing tests pass, clippy clean

### 2026-03-09 — spec scan (run 674)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 9 tracked Gloas PRs still OPEN (#4979, #4962, #4960, #4940, #4939, #4932, #4843, #4840, #4630)
- CI in progress for run 671 push (check+clippy+ef_tests passed, 4 jobs still running)
- cargo audit: same 1 medium rsa advisory (transitive, no fix)
- No code changes needed — spec stable

### 2026-03-09 — spec scan (run 673)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 9 tracked Gloas PRs still OPEN (#4979, #4962, #4960, #4940, #4939, #4932, #4843, #4840, #4630)
- CI green (latest completed), nightly 3 consecutive greens (Mar 7-9)
- cargo audit: same 1 medium rsa advisory (transitive, no fix)
- Test coverage audit: all Gloas code paths comprehensively tested (20k+ lines of Gloas tests)
- No code changes needed — spec stable

### 2026-03-09 — spec scan (run 672)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- Recent consensus-specs commits: CI/tooling only (release-drafter v6.4.0, strategy matrix, sampling config fix, dependency cleanup, EIP-6800 removal)
- All 9 tracked Gloas PRs still OPEN (#4979, #4962, #4960, #4940, #4939, #4932, #4843, #4840, #4630)
- PR #4979 (PTC Lookbehind) still blocked (mergeable_state=blocked, last activity Mar 7)
- CI in progress from run 671 push — all 6 jobs running
- cargo audit: same 1 medium rsa advisory (transitive, no fix)
- No code changes needed — spec stable

### 2026-03-09 — spec scan (run 669)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 9 tracked Gloas PRs still OPEN (#4979, #4962, #4960, #4940, #4939, #4932, #4843, #4840, #4630)
- PR #4979 (PTC Lookbehind) still blocked (last activity Mar 7), PR #4954 (fork choice ms) blocked (last activity Mar 2)
- CI green: ci passed, spec-test-version-check passed, clippy clean
- cargo audit: same 1 medium rsa advisory (transitive, no fix)
- No code changes needed — spec stable

### 2026-03-09 — spec scan + codebase audit (run 666)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 11 tracked Gloas PRs still OPEN (dropped #4747 Fast Confirmation Rule — not core ePBS)
- PR #4979 (PTC Lookbehind) still blocked, last activity Mar 7
- CI green: all 7 jobs passed; nightly 5 consecutive greens (Mar 5-9)
- Deep codebase audit: zero TODO/FIXME/HACK/XXX in Rust source, zero compiler warnings, zero clippy warnings
- Explored untested error paths: BuilderPaymentIndexOutOfBounds and BitFieldError in envelope_processing are defensive checks for structurally impossible states (Vector always correctly sized) — low-value to test
- cargo audit: unchanged (1 medium rsa advisory, transitive, no fix)
- No code changes needed — spec stable, codebase clean

### 2026-03-09 — spec scan + PR 4979 readiness review (run 665)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release, no new Gloas PR merges
- All 12 tracked Gloas PRs still OPEN
- CI green: 4/6 jobs passed, 2 in progress; nightly 3 consecutive greens (Mar 7-9)
- PR #4979 (PTC Lookbehind) most active: 10 review comments from jtraglia, potuz, ensi321, nflaig (Mar 6-7); adds `ptc_lookbehind` field to BeaconState (`Vector[Vector[ValidatorIndex, PTC_SIZE], 2 * SLOTS_PER_EPOCH]`), new `compute_ptc` helper (extracted from `get_ptc`), `get_ptc` becomes cache lookup, new `process_ptc_lookbehind` epoch processing step, `initialize_ptc_lookbehind` for fork upgrade/genesis. Not yet approved — monitoring.
- Verified our `get_ptc_committee` matches what will become `compute_ptc` in the PR; epoch processing ordering ready for insertion of `process_ptc_lookbehind` after `process_proposer_lookahead`
- cargo audit: 1 vulnerability (RUSTSEC-2023-0071 rsa, medium, no fix), 5 allowed warnings — all transitive, unchanged
- No code changes needed — spec stable

### 2026-03-09 — fork transition audit + performance review (run 664)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release, no new Gloas PR merges
- All 12 tracked Gloas PRs still OPEN
- CI green: latest ci run in_progress, previous green; nightly greens continue
- Audited fork transition edge cases (Fulu→Gloas boundary): upgrade_to_gloas field initialization, load_parent latest_block_hash patching, get_advanced_hot_state cache-hit path at fork boundary — all correctly handled through fork_name checks, error-based fallbacks, and zero-block-hash guards
- Performance review of Gloas hot paths: identified self-build envelope double-processing (build_self_build_envelope + process_self_build_envelope), payload attestation aggregation clones, bid pool insert clones — all negligible in practice (once-per-12s block production path)
- Zero compiler warnings, cargo audit unchanged (1 medium rsa transitive advisory)
- No code changes needed — codebase clean, spec stable

### 2026-03-09 — deep audit of Gloas production code paths (run 663)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All tracked Gloas PRs still OPEN; PR #4979 (PTC Lookbehind) and #4954 (fork choice milliseconds) still unmerged
- CI green: latest run in_progress (previous 2 green), nightly 3 consecutive greens (Mar 7-9)
- Deep audit of Gloas production code: envelope_processing.rs, execution_bid_pool.rs, gloas_verification.rs, beacon_chain.rs envelope/bid paths, fork choice weight calculation
- Verified: `can_builder_cover_bid` correctly includes MIN_DEPOSIT_AMOUNT floor, builder payment index calculation bounds are safe, `process_execution_payload_envelope` state mutations are correct, fork choice `get_gloas_weight`/`should_extend_payload`/`find_head_gloas` logic matches spec
- Zero clippy warnings, zero compiler warnings across workspace
- No code changes needed — codebase clean, spec stable

### 2026-03-09 — gossip verification test coverage (run 662)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 12 tracked Gloas PRs still OPEN; all recently merged PRs are maintenance/CI only
- Added `attestation_invalid_signature_does_not_poison_observation_cache` test to gloas_verification.rs — verifies that a bad-sig payload attestation doesn't mark PTC validators as "already seen", which would block subsequent valid attestations. Mirrors the existing bid poison test.
- CI green, nightly 5+ consecutive greens

### 2026-03-09 — production code audit + spec scan (run 661)
- Spec stable: no new consensus-specs release (v1.7.0-alpha.2), no new spec-test release (v1.5.0), no new Gloas PR merges
- All 12 tracked Gloas PRs still OPEN; recent merged PRs are maintenance only (#4991 CI matrix, #4990 release-drafter, etc.)
- Production code safety audit: zero `.unwrap()` or `.expect()` in consensus state_processing production code (all in test modules only); `dump_as_dot` debug function in beacon_chain.rs has unwraps but is non-consensus
- CI green: latest ci run passed (commit 7234b3e24); docker builds queued (waiting for runners)
- No code changes — codebase clean, spec stable

### 2026-03-09 — spec scan (run 659)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- PR #4979 (PTC Lookbehind) still OPEN, only tracked PR not yet merged
- NEW: PR #4950 (extend by_root reqresp serve range to match by_range) merged Mar 6 — analyzed: vibehouse by_root handlers already serve all stored data without range restriction, exceeding the minimum spec requirement. No code change needed.
- NEW: PR #4954 (fork choice store milliseconds) OPEN — converts fork choice timing from seconds to milliseconds, related to merged #4926 (SLOT_DURATION_MS). Not merged yet, monitor.
- CI: latest run (commit 7234b3e24) in progress; previous CI green; nightly passed (Mar 9)
- No code changes needed — spec stable, fully compliant

### 2026-03-09 — spec scan (run 658)
- All tracked Gloas PRs still OPEN; PR #4979 (PTC Lookbehind) still needs reviews
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- Verified 6 recently merged Gloas PRs (#4948, #4930, #4926, #4922, #4920, #4916) — all already implemented or not applicable
- CI: latest run in progress, last completed green; nightly 3 consecutive greens (Mar 7-9)
- Zero clippy warnings, zero compiler warnings across entire workspace
- cargo audit: same 1 medium rsa advisory (transitive, no fix available)
- Audited per_epoch_processing/gloas.rs test coverage: 15+ builder pending payment tests, per_block_processing/gloas.rs has 199 tests, envelope_processing has 55 tests — all comprehensive
- No code changes needed — spec stable, codebase clean

### 2026-03-09 — spec scan (run 657)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- CI: ef-tests + check/clippy/fmt green, remaining jobs in progress
- Verified PRs #4918 (attestation payload status check) and #4923 (ignore blocks with unknown parent payload) are both fully implemented with tests
- Audited envelope_processing.rs test coverage: 50+ tests covering all error variants, happy paths, state mutations, signature verification, execution requests, and payment queueing — no gaps found
- No code changes needed

### 2026-03-09 — spec scan (run 656)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- CI green: ci passed, nightly 3 consecutive greens (Mar 7-9), spec-test-version-check passed
- cargo audit: same 1 medium rsa advisory (transitive, no fix available)
- Deep audit of test gaps: epoch processing (process_builder_pending_payments), deposit routing (process_deposit_request_gloas), withdrawal processing all have comprehensive coverage; remaining gaps are defense-in-depth unreachable paths (e.g. Hash256 always 32 bytes, BitVector index always valid)
- No code changes needed

### 2026-03-09 — test coverage (run 655)
- Spec stable, no new merges. CI green.
- Audited Gloas error paths for test gaps; found validator sweep BLS credential path and attestation slot overflow untested
- Added 3 new tests: validator sweep BLS-credential skip behavior (mutable + read-only paths), payload attestation slot overflow at u64::MAX
- Discovery: `WithdrawalCredentialsInvalid` error at lines 657/667 in process_withdrawals_gloas is defense-in-depth only — `is_fully_withdrawable_validator`/`is_partially_withdrawable_validator` already filter out BLS-credential validators via `has_execution_withdrawal_credential`
- All 199 gloas tests pass, clippy clean, pushed

### 2026-03-09 — spec scan (run 650)
- All 12 tracked Gloas PRs still OPEN, no new merges
- Found 2 additional Gloas-labeled PRs: #4747 (Fast Confirmation Rule, eip7805/FOCIL) and #4558 (Cell Dissemination, fulu+gloas) — neither is core ePBS, no action needed
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- Recent merged PRs: maintenance only (dependency updates, CI improvements, EIP-6800/7441 removal)
- CI: all 6 jobs green (check+clippy+fmt, ef-tests, unit-tests, beacon-chain, http-api, network+op_pool)
- Nightly: spec-test-version-check passed (no new release)
- cargo audit: same 1 medium rsa advisory (transitive, no fix available)
- No code changes — spec stable, fully compliant

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
