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

### run 1157 (Mar 13) — attestation clone + ancestor cache
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. cargo audit unchanged (1 rsa, 5 allowed warnings). Nightly tests green. No semver-compatible dep updates.

Shipped two optimizations:
1. Eliminated unnecessary `attestation.clone()` in `SplitAttestation::new` — the function already takes ownership of the attestation, so it can be destructured directly by consuming the match. Also avoids a redundant `signature.clone()` by moving the field directly from the variant. All 36 operation_pool tests pass.
2. Added ancestor lookup cache in `get_gloas_weight` — many validators vote for the same block root, causing `get_ancestor_gloas(root, slot)` to repeat the same O(depth) tree walk for each validator. Now caches results per `vote.current_root` within each weight calculation, avoiding redundant walks. All 188 proto_array + 119 fork_choice + 8/8 EF fork choice spec tests pass. Clippy clean.

### run 1156 (Mar 13) — zero-clone maximum_cover
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. cargo audit unchanged (1 rsa, 5 allowed warnings). Nightly tests green. Rebased ptc-lookbehind branch (17 commits behind), 575/575 state_processing tests pass on branch.

Shipped: eliminated clone in `maximum_cover` greedy selection loop. Previously, each selected item was cloned (including its entire covering set HashMap) so it could be referenced while updating other items. Now uses `split_at_mut` to borrow the best item in-place while mutating others, and `Option::take()` to move items into the result instead of cloning. Removed `Clone` bounds from `MaxCover` trait and associated types. For attestation packing, this eliminates one `HashMap<u64, u64>` clone per selected attestation per iteration (up to 128 per epoch). Also updated transitive deps (anstyle 1.0.14, colorchoice 1.0.5). All 36 operation_pool tests pass. Clippy clean.

### run 1155 (Mar 13) — cached attestation reward sum
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. PR #4940 (Gloas fork choice tests) merged — test generators only, no new vectors yet. cargo audit unchanged (1 rsa, 5 allowed warnings, no fixes).

Shipped: cached `reward_numerator_sum` field in `AttMaxCover` — `score()` now returns the pre-computed sum instead of iterating the entire `fresh_validators_rewards` HashMap on every call. The cached sum is initialized at construction and decremented in `update_covering_set` when validators are removed. This eliminates O(n) HashMap value iteration per `score()` call (where n = remaining fresh validators per attestation). Combined with run 1154's call reduction, attestation scoring during max cover is now O(1) per item. All 36 operation_pool tests pass. Clippy clean.

### run 1154 (Mar 13) — max_cover score() optimization
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. No semver-compatible dep updates available. cargo audit unchanged (1 rsa, no fix).

Shipped: optimized `maximum_cover` in operation_pool — reduced redundant `score()` calls from 3× per item per iteration to 1×. For attestations, `score()` sums a HashMap of validator rewards, so this eliminates ~2n HashMap iterations per outer loop step. Also removed score check from update pass (update_covering_set on empty set is a no-op). All 36 operation_pool tests pass. Clippy clean.

### run 1153 (Mar 13) — all stable, Cargo.lock fix
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. v1.7.0-alpha.3 tag exists but no release published yet. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. PR #4939 updated (2026-03-13). No new Gloas PRs beyond existing tracked set.

Shipped: committed Cargo.lock with smallvec dependency for operation_pool (was added as dep but lockfile not committed). Audited consensus hot paths for optimization opportunities — codebase is in good shape. All production `collect()`, `clone()`, and `HashMap` usages are either necessary (borrow conflicts with mutation) or in test code. Clippy clean across state_processing, proto_array, fork_choice, beacon_chain.

### run 1150 (Mar 13) — bitlist_extend bulk byte optimization
Spec stable: no new consensus-specs commits, releases, or spec-test vectors. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. PR #4992 same head d76a278b0a.

Shipped: optimized `bitlist_extend` in operation_pool attestation storage — replaced O(n) bit-by-bit iteration (with bounds check per bit) with bulk byte copy + shift-OR for non-aligned cases. Added 5 unit tests (byte-aligned, non-aligned, empty, all-set, overflow). All 36 operation_pool tests pass. Clippy clean.

### run 1149 (Mar 13) — all stable, monitoring
Spec stable: no new consensus-specs commits since #5004 (Mar 13). No new releases (still v1.7.0-alpha.2 published), no new spec-test vectors (still v1.6.0-beta.0). All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. PR #4992 (PTC lookbehind): OPEN, MERGEABLE, 1 APPROVED (jtraglia), same head d76a278b0a. CI green (clippy passed, other jobs running). cargo audit unchanged (1 rsa, no fix available). No semver-compatible dep updates. No outdated direct deps (only rand dev dep version mismatch in network tests). moonrepo/setup-rust@v1 Node.js 20 deprecation warning — upstream hasn't published Node.js 24 version yet, nothing actionable.

### run 1148 (Mar 13) — hdiff encode buffer optimization
Spec stable: no new releases (still v1.7.0-alpha.2 published), no new spec-test vectors (still v1.6.0-beta.0). Tracked PRs (#4932, #4939, #4960, #4962, #4992) all OPEN. PR #4940 (Gloas fork choice tests) merged — test generators only, no new vectors. CI green. cargo audit unchanged (1 rsa, no fix available).

Shipped: reduced xdelta3 encode buffer over-allocation in `compute_xdelta` — initial buffer now 1/4 of total size (was 2x), with retry-on-resize matching the existing `apply_xdelta` pattern. Added `store_hdiff_buffer_compute_resizes` metric. All 30 store tests + 7 hdiff tests pass.

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
