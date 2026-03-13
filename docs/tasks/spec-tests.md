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

### run 1022 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). PR #4747 (Fast Confirmation Rule): updated today, still dirty — not ready. PR #5002 (wording cleanup): still open, no code impact. CI in progress (previous run green). Nightly green (3+ consecutive). No new issues or PRs on dapplion/vibehouse. ptc-lookbehind branch 2 doc-only commits behind main. cargo audit unchanged (1 rsa).

### run 1021 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). PR #4747 (Fast Confirmation Rule): updated today, still CONFLICTING/REVIEW_REQUIRED — not ready. CI in progress (check+clippy+ef-tests passed, others running). Nightly green. No new issues or PRs on dapplion/vibehouse. ptc-lookbehind branch 1 doc-only commit behind main. No compatible dep updates. cargo audit unchanged (1 rsa).

### run 1016 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.5.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED from jtraglia, MERGEABLE). PR #4747 (Fast Confirmation Rule): updated today but still dirty/early-stage. CI green. Nightly green. No new issues or PRs on dapplion/vibehouse. ptc-lookbehind branch 5 doc-only commits behind main — clean merge when #4992 lands.

### run 1015 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED from jtraglia, MERGEABLE). CI green. Nightly green. Zero clippy warnings. Audited envelope_processing.rs test coverage — all 12 EnvelopeProcessingError variants have dedicated test coverage (40+ tests total). No gaps found. No dependency updates available.

### run 1014 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED from jtraglia, MERGEABLE). CI green. Nightly green (3+ consecutive). No new issues or PRs on dapplion/vibehouse. Docker build in progress.

### run 1013 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED from jtraglia, MERGEABLE). CI green. Nightly green. No new issues on dapplion/vibehouse.

### run 1012 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED from jtraglia, MERGEABLE). PR #5002 (wording cleanup, no code impact, still open). CI green. Nightly green. No new issues on dapplion/vibehouse. ptc-lookbehind branch 2 doc-only commits behind main — clean merge when #4992 lands.

### run 1011 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED from jtraglia, MERGEABLE). PR #5002 (wording cleanup, no code impact, still open). CI green. Nightly green. No new issues on dapplion/vibehouse. ptc-lookbehind branch 2 doc-only commits behind main — clean merge when #4992 lands.

### run 1010 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED from jtraglia, MERGEABLE). PR #5002 (wording cleanup, no code impact). PR #4747 (Fast Confirmation Rule) updated but still dirty/early-stage. CI green. Nightly green. cargo audit unchanged (1 rsa, 5 warnings). Docker builds queuing (runner availability). No outdated deps except major version bumps (rand 0.8→0.9). No actionable TODOs in Gloas code.

### run 1009 (Mar 13) — no spec changes, rebased ptc-lookbehind
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED from jtraglia, MERGEABLE). PR #5002 (wording cleanup, no code impact) still open. Rebased ptc-lookbehind branch onto main (was 20 commits behind), 575/575 state_processing tests pass, pushed. Nightly in progress (http-api-tests remaining, all others passed). CI green. cargo audit unchanged (1 rsa, 5 warnings).

### run 1008 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED from jtraglia, MERGEABLE). PR #5002 (wording cleanup) new. CI green. Nightly in progress (http-api-tests remaining, all others passed). cargo audit unchanged (1 rsa, 5 warnings). Audited Gloas production code quality: no unwraps, all safe arithmetic, comprehensive integration test coverage across all critical paths.

### run 1005 (Mar 13) — v1.7.0-alpha.3 audit, all changes already implemented
Spec bumped to v1.7.0-alpha.3 (d2cfa51c, Mar 11). Audited all gloas changes between alpha.2 and alpha.3: PayloadStatus reorder (EMPTY=0,FULL=1,PENDING=2) ✓, is_pending_validator + deposit routing ✓, payload_data_availability_vote dual tracking ✓, should_extend_payload requires DA ✓, validate_on_attestation index=1 check ✓, P2P bid parent_block_root filtering ✓, envelope serve range ✓. All changes confirmed in codebase (see docs/tasks/spec-update-post-alpha2.md). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). No new spec-test vectors (still v1.6.0-beta.0). CI green. Nightly in progress. cargo audit unchanged (1 rsa, 5 warnings).

### run 1004 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). CI green. Nightly in progress. cargo audit unchanged (1 rsa, 5 warnings). No dependency updates available. No open issues or PRs requiring action.

### run 1002 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). CI green. Nightly in progress. cargo audit unchanged (1 rsa, 5 warnings). No dependency updates available. No open issues or PRs requiring action.

### run 1000 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). CI green. Nightly green (3+ consecutive). cargo audit unchanged (1 rsa, 5 warnings). No dependency updates available. No open issues or PRs requiring action.

### run 999 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). CI green. Nightly green (3+ consecutive). cargo audit unchanged (1 rsa, 5 warnings). No open issues or PRs requiring action.

### run 998 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). CI green. Nightly green (4 consecutive). cargo audit unchanged (1 rsa, 5 warnings). No open issues or PRs requiring action.

### run 997 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). CI green. Nightly green (4 consecutive). cargo audit unchanged (1 rsa, 5 warnings). clippy clean on consensus crates. Docker build queued (runner availability). No open issues or PRs requiring action.

### run 996 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). CI green. cargo audit unchanged (1 rsa, 5 warnings). No open issues or PRs requiring action.

### run 995 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). PR #4939: still OPEN, no approvals, updated today. CI green. cargo audit unchanged (1 rsa, 5 warnings). No open issues requiring action.

### run 994 (Mar 13) — no spec changes, code audit clean
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2, GitHub latest=v1.6.1). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE, labels: testing+gloas+heze). Ran code audit on all Gloas consensus paths (gloas.rs, envelope_processing.rs, fork_choice.rs, proto_array, gloas_verification.rs, block_verification.rs): no bugs, no unsafe arithmetic, no unwraps in production code, no TODOs. All 56 envelope processing tests cover every error variant. CI green. cargo audit unchanged (1 rsa, 5 warnings). 0 compatible dep updates.

### run 992 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2, GitHub latest=v1.6.1). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE, head d76a278b0a). All 10 tracked Gloas PRs unchanged. PR #4939 updated Mar 13 (rebase, no semantic changes). CI in progress (4/6 jobs passed, 2 running). Nightly green (3 consecutive). cargo audit unchanged (1 rsa). clippy clean on consensus crates. ptc-lookbehind 3 doc-only commits behind main, clean merge. No outdated deps (only major version bumps available: rand 0.8→0.9).

### run 991 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). All tracked Gloas PRs unchanged. CI in progress from last push. Nightly green (3 consecutive). cargo audit unchanged (1 rsa). Audited test coverage across state_processing — all high-priority areas (withdrawal OOB, builder index validation, payload attestation, epoch payment processing, per-slot availability, PTC committee, deposit routing, envelope processing) have comprehensive edge case tests. ptc-lookbehind branch 2 commits behind main.

### run 990 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). PR #4940 (fork choice tests): file rename only, no semantic changes. PR #4939: rebased on master, no semantic changes. Noted PR #4747 (Fast Confirmation Rule, mkalinin) — labeled gloas, early-stage proposal, dirty mergeable, not ready for implementation. CI in progress (check+clippy+ef-tests+network+op_pool passed). Nightly green (3 consecutive). cargo audit unchanged (1 rsa).

### run 989 (Mar 13) — rebased ptc-lookbehind, no spec changes
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). Rebased `ptc-lookbehind` branch onto main (19 commits behind → clean, 575/575 state_processing tests pass). CI in progress (check+clippy passed). Nightly green (3 consecutive). cargo audit unchanged (1 rsa).

### run 988 (Mar 13) — SSZ round-trip tests for proto_array Gloas fields
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). CI green. Nightly green (3 consecutive). cargo audit unchanged (1 rsa). Added 5 SSZ round-trip tests verifying all Gloas ePBS fields survive encode/decode on ProtoNode, VoteTracker, and full SszContainer (restart correctness). Added Debug derive to VoteTracker.

### run 987 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). CI green. Nightly green (3 consecutive). cargo audit unchanged (1 rsa).

### run 984 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). All tracked PRs unchanged. CI green. Nightly green (4 consecutive, Mar 10 failure was pre-fix code). cargo audit unchanged (1 rsa).

### run 983 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). All tracked PRs unchanged. CI green. Nightly green (3 consecutive). cargo audit unchanged (1 rsa).

### run 982 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). PR #4939: updated (push, no semantic changes). CI green. cargo audit unchanged (1 rsa).

### run 979 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). All tracked Gloas PRs unchanged. No dependency updates available. CI green. Nightly green (6+ consecutive). cargo audit unchanged (1 rsa).

### run 974 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, MERGEABLE). All tracked Gloas PRs unchanged. No dependency updates available. CI green. Nightly green (4 consecutive). cargo audit unchanged (1 rsa).

### run 968 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, head d76a278b0a, mergeable=clean). PR #4939 rebased on master (fdfad73e31, merge commit Mar 13), no semantic changes. All 10 tracked Gloas PRs unchanged. No dependency updates available (all at latest compatible). CI green. Nightly green (3 consecutive). cargo audit unchanged (1 rsa, 5 warnings).

### run 965 (Mar 13) — no spec changes, all stable
No new spec commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): still OPEN (1 APPROVED, head d76a278b0a, mergeable=clean). All tracked PRs (#4939, #4940, #5002) unchanged. No dependency updates available. CI green. Nightly green (4 consecutive). cargo audit unchanged (1 rsa, 5 warnings).

### run 964 (Mar 13) — no spec changes, PR #4939 wording update
No new spec commits, release, or test vectors. PR #4992 still OPEN (1 APPROVED, head d76a278b0a). PR #4939 got wording update (Mar 13) — no semantic changes, our implementation still matches. PR #4940 (fork choice tests) updated Mar 12 — our handler supports `on_execution_payload` steps and `head_payload_status` checks. CI green. Nightly green. cargo audit unchanged.

### run 963 (Mar 13) — rebased ptc-lookbehind, all stable
No new spec commits, release, or test vectors. PR #4992 still OPEN (1 APPROVED). Rebased `ptc-lookbehind` branch onto main (37 commits behind → clean, 575/575 tests pass). CI green. Nightly green (3 consecutive). cargo audit unchanged.

### run 962 (Mar 13) — no spec changes, all stable
Spec scan: no new consensus-specs commits since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2 on GitHub). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged, 1 APPROVED (jtraglia Mar 12), still OPEN. PR #5002 still OPEN. All other tracked PRs unchanged. CI green. Nightly green. Docker build queued (runner scheduling). cargo audit: 1 rsa advisory (no fix) + 5 allowed. No dependency patch updates available (all pending are major version bumps).

### run 960 (Mar 13) — consolidated spec-update-post-alpha2.md, no spec changes
Spec scan: no new consensus-specs commits since #5001 (Mar 12). No new spec release (v1.7.0-alpha.3 version bumped but not released on GitHub). No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged, 1 APPROVED (jtraglia Mar 12), still OPEN. All other tracked PRs still OPEN. CI green. Nightly green. cargo audit: 1 rsa advisory (no fix) + 5 allowed. Consolidated spec-update-post-alpha2.md progress log from 445→~140 lines.

### run 959 (Mar 13) — alpha.3 version bump, all changes already implemented
consensus-specs bumped to v1.7.0-alpha.3 (#4999, Mar 11). Reviewed all commits between alpha.2 and alpha.3 — all consensus changes already implemented in vibehouse: #5001 (parent_block_root bid filtering — already had 3-tuple key), #4918 (attestation payload status check), #4923 (ignore block if parent payload unknown), #4897 (is_pending_validator deposit check), #4916 (deposit request refactor), #4948 (payload status constant reorder), #4930 (payload_states rename — naming-only), #4914 (execution proof validator_index — our own proof type). PR #4915 (proof dedup optimization) noted as future improvement. No new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind) now has 1 APPROVED (jtraglia Mar 12). New PR #5002 (wording clarification only). EF tests: 138/138 fake_crypto, 8/8 fork choice. cargo audit: 1 rsa advisory (no fix). CI green.

### runs 759-958 consolidated (Mar 10-13) — spec stable, no code changes needed
Spec completely stable — no new consensus-specs commits with consensus changes since #5001 (Mar 12), no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). All 11 tracked Gloas PRs remained OPEN throughout. PR #4992 (PTC lookbehind) evolved from head 215962a9 (blocked) to d76a278b0a (clean, 1 APPROVED jtraglia Mar 12). CI and nightly continuously green. EF spec tests: consistently 35/35 (minimal, fake_crypto), fork choice: 8/8 (real crypto). Workspace tests: 2643/2651 (8 web3signer timeout). cargo audit: 1 rsa advisory (no fix). Recent consensus-specs merges were all CI/tooling (#4984 remove Verkle, #4988-#4995 Python/reftest/release-drafter).

Notable activities:
- Run 929: Implemented PTC lookbehind on branch `ptc-lookbehind` (previous_ptc/current_ptc fields, compute_ptc, get_ptc cached reads, per_slot rotation, upgrade initialization). All 575 state_processing tests pass. NOT merged — blocks on PR #4992 merge + new spec test vectors.
- Run 929: Fixed `clippy::large_stack_frames` in `proposer_boost_re_org_test` (Rust 1.91 bump)
- Run 926: Updated deps (clap 4.6, openssl 0.10.76, c-kzg 2.1.7, tempfile 3.27)
- Run 871: Updated Cargo.lock (windows-sys transitive deps)
- Run 850: Added workflow_dispatch trigger to ci.yml
- Run 834: Codebase audit — 39 TODOs (all inherited/spec-dependent), gloas.rs has 208 unit tests across 9216 lines
- Run 800: Analyzed PTC lookbehind implementation plan (7 code areas)
- Run 760: Discovered PR #4962 (sanity/blocks tests)

### 2026-03-09 — consolidated: runs 642-744 (Mar 9)
Spec completely stable throughout — no new consensus-specs merges with consensus changes, no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). All tracked Gloas PRs remained OPEN. CI and nightly continuously green (5+ consecutive). cargo audit: 1 rsa advisory (no fix).

Key code changes during this period:
- **run 735**: Fixed 2 beacon_chain test failures (slasher backend guard, Fulu fork scheduling check) — 766→768/768
- **run 723**: Added 15 proto_array propagation tests (validation/invalidation/viability — 6+6+3)
- **run 725**: Added 7 tests for `contains_invalid_payloads` + `on_invalid_execution_payload`
- **run 718**: Deep spec conformance audit — all Gloas functions verified correct against consensus-specs master
- **run 717**: Added 6 tests for `process_payload_attestation` + `get_indexed_payload_attestation`
- **run 701**: Implemented PR #4939 (index-1 attestation envelope validation) proactively
- **run 692**: Tracked 3 approved PRs (#4843, #4898, #4892) ready to merge
- **run 683**: Discovered new PR #4992 (PTC lookbehind minimal 2-slot alternative to #4979)
- **run 681**: Analyzed PTC lookbehind implementation requirements
- **run 680**: Updated zerocopy 0.8.41→0.8.42
- **run 678**: Added `ptc_committee_no_validators_returns_error` test
- **run 677**: Updated alloy-trie, quinn-proto, yamux patch deps
- **run 676**: Added 4 prometheus gauge metrics for ePBS pool monitoring
- **run 675**: Added 5 epoch processing integration/edge case tests
- **run 665**: Analyzed PR #4979 (PTC lookbehind) design in detail
- **run 662**: Added attestation signature poison cache test
- **run 659**: Analyzed PR #4950 (extend by_root serve range) — already compliant
- **run 655**: Added 3 tests (BLS credential skip, slot overflow)
- **run 643**: Expanded tracker to 12 Gloas PRs

Extensive audits across these runs confirmed: zero unwrap() in consensus production code, zero todo!()/unimplemented!() in production, all Gloas functions match spec, 90%+ test coverage across all production code, 9200+ lines of Gloas tests in state_processing alone.

### 2026-03-09 — consolidated: runs 524-641 (Mar 7-9)
Key activities across ~120 runs (routine scans omitted):
- **run 641**: docker CI paths-ignore for docs-only commits
- **run 640**: post-rebrand devnet verification SUCCESS
- **run 762**: spec scan — no new merges/releases, 35/35 EF tests pass, clippy clean, CI green
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
