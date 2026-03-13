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

### run 936 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged (head d76a278b0a, clean). Still OPEN. Branch `ptc-lookbehind` ready.
- **PR #4939 (index-1 attestation)**: head fdfad73e31, updated Mar 13. Still OPEN.
- All other tracked PRs (#4954, #4843, #4898, #4892, #4940, #4932, #4960, #4962, #5002): all still OPEN.
- CI run 23035917712 in_progress: check+clippy+fmt ✓, ef-tests ✓, network+op_pool ✓, beacon_chain/http_api/unit tests still running.
- cargo audit unchanged (1 vuln + 5 allowed). No compatible dep updates (0 lockfile changes).
- No code changes needed.

### run 935 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged (head d76a278b0a, clean). Still OPEN. Branch `ptc-lookbehind` ready.
- **PR #4939 (index-1 attestation)**: head fdfad73e31, updated Mar 13 (likely rebase). Still OPEN.
- All other tracked PRs (#4954, #4843, #4898, #4892, #4940, #4932, #4960, #4962, #5002): all still OPEN.
- CI run 23035917712 in_progress: check+clippy+fmt ✓, ef-tests ✓, network+op_pool ✓, beacon_chain/http_api/unit tests still running.
- cargo audit unchanged (1 vuln + 5 allowed). No compatible dep updates (0 lockfile changes).
- No code changes needed.

### run 934 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged (head d76a278b0a, clean). Still OPEN. Branch `ptc-lookbehind` ready.
- **PR #4939 (index-1 attestation)**: head fdfad73e31, blocked. Still OPEN.
- All other tracked PRs (#4954, #4843, #4898, #4892, #4940, #4932, #4960, #4962, #5002): all still OPEN.
- CI run 23035917712 in_progress: check+clippy+fmt ✓, ef-tests ✓, network+op_pool ✓, beacon_chain/http_api/unit tests still running.
- cargo audit unchanged (1 vuln + 5 allowed). No compatible dep updates (0 lockfile changes).
- No code changes needed.

### run 933 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged (head d76a278b0a). Still OPEN. Branch `ptc-lookbehind` ready.
- **PR #4939 (index-1 attestation)**: updated Mar 13, still OPEN.
- All other tracked PRs (#4954, #4843, #4898, #4892, #4940, #4932, #4960, #4962, #5002): all still OPEN.
- CI run 23035917712 in_progress for commit 2f3aaf9: check+clippy+fmt ✓, ef-tests ✓, network+op_pool ✓, beacon_chain/http_api/unit tests still running.
- cargo audit unchanged (1 vuln + 5 allowed). No compatible dep updates.
- No code changes needed.

### run 932 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #5001 (parent_block_root in bid filtering key)**: merged Mar 12. Verified vibehouse already implements the 3-tuple `(slot, parent_block_hash, parent_block_root)` key in `ObservedExecutionBids::is_highest_value_bid()` — no code change needed.
- **PR #4992 (PTC lookbehind)**: unchanged (head d76a278b0a, mergeable=clean, 1 APPROVED jtraglia Mar 12). Still OPEN. Branch `ptc-lookbehind` ready to merge once upstream lands + new spec test vectors released.
- All other tracked PRs (#4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962, #5002): all still OPEN.
- CI in_progress for latest commit (386bb7d). EF tests (fake_crypto + real crypto) and check+clippy+fmt passed. Nightly green. cargo audit unchanged (1 vuln + 5 allowed). No compatible dep updates.
- No code changes needed.

### run 930 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged (head d76a278b0a, mergeable=clean, 1 APPROVED jtraglia Mar 12). Still OPEN.
- All other tracked PRs (#4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962): all still OPEN.
- CI in_progress for latest commit (2f3aaf9). cargo audit unchanged (1 vuln + 5 allowed). No compatible dep updates.
- No code changes needed.

### run 929 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged (head d76a278b0a, mergeable=clean, 1 APPROVED jtraglia). Still OPEN.
- **Implemented PTC lookbehind on branch `ptc-lookbehind`**: Added `previous_ptc`/`current_ptc` fields to BeaconStateGloas, renamed `get_ptc_committee`→`compute_ptc`, added `get_ptc` (cached reads), rotation in `per_slot_processing`, initialization in `upgrade_to_gloas`. All 575 state_processing unit tests pass. **NOT merged to main** — blocks on PR #4992 merge + new spec test vectors (SSZ layout change breaks EF test parsing).
- Fixed pre-existing `clippy::large_stack_frames` error in `proposer_boost_re_org_test` (introduced by Rust 1.91 bump in PR #30).
- Other tracked PRs (#4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962): all still OPEN.

### run 926 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (v1.7.0-alpha.3 committed but GitHub release still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- PR #4992 (PTC lookbehind): major update — 8 commits, head d76a278b0a, mergeable=clean. Design: two separate state fields `previous_ptc`/`current_ptc` instead of vector. Approaching merge.
- New PR #5002 (p2p wording, no consensus impact). All other tracked PRs still OPEN.
- Updated deps (clap 4.6, openssl 0.10.76, c-kzg 2.1.7, tempfile 3.27). Build clean, types 715/715 pass.
- CI in_progress. Nightly green. cargo audit unchanged (1 vuln + 5 allowed).

### run 924 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, last activity Mar 10 12:05 UTC). CI green, nightly green. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 923 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, last activity Mar 10 12:05 UTC). CI green, nightly green. 0 compatible dep updates. No code changes needed.

### run 921 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, last activity Mar 10 12:05 UTC). CI green, nightly green. 0 compatible dep updates. cargo audit unchanged. No code changes needed.

### run 916 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, last activity Mar 10 12:05 UTC). CI green, nightly green. 0 compatible dep updates. cargo audit unchanged. No code changes needed.

### run 915 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, last activity Mar 10 12:05 UTC). CI green, nightly green. 0 compatible dep updates. cargo audit unchanged. No code changes needed.

### run 909 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=true, no new comments since Mar 10 12:05 UTC)
- Nightly 22894949404 failure was on stale commit 0d12a857 (pre race-fix); nightly 22908449717 on current HEAD: 24/26 jobs passed incl. network-tests (fulu) SUCCESS, http-api fulu/electra still running
- CI green. 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed

### run 898 (Mar 10)
- Spec scan: no new consensus-specs commits since run 889. All 12 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- CI green (run 22905618379). Nightly running on latest HEAD. 0 compatible dep updates. No code changes needed

### run 889 (Mar 10)
- Spec scan: no new consensus-specs commits since run 888. All 11 tracked PRs still OPEN (10 Gloas-related). No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- CI fully green (run 22905618379: all 7 jobs passed). Docker build 22905618349 still queued
- Nightly Mar 10 failed on pre-fix commit (0d12a85) — fix in HEAD, tonight's nightly will pass
- 0 compatible dep updates. No code changes needed

### run 888 (Mar 10)
- Spec scan: no new consensus-specs commits since run 887. All 11 tracked PRs still OPEN (10 Gloas-related). No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- CI fully green (run 22905618379: all 7 jobs passed). Docker build 22905618349 still queued
- Nightly Mar 10 failed on pre-fix commit (0d12a85) — fix 62df568 in HEAD, tonight's nightly will pass
- 0 compatible dep updates. No code changes needed

### run 887 (Mar 10)
- Spec scan: no new consensus-specs commits since run 886. All 11 tracked PRs still OPEN (10 Gloas-related). No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- CI fully green (run 22905618379: all 7 jobs passed). Docker build 22905618349 still queued
- 0 compatible dep updates. No code changes needed

### run 886 (Mar 10)
- Spec scan: no new consensus-specs commits since run 885. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- CI fully green (run 22905618379: all 7 jobs passed). Docker build 22905618349 still queued
- 0 compatible dep updates. No code changes needed

### run 885 (Mar 10)
- Spec scan: no new consensus-specs commits since run 884. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- Recently merged PRs: unchanged (latest: #4995 Python 3.14 support — CI/tooling only)
- CI fully green (run 22905618379: all 7 jobs passed). Nightly Mar 10 failure was pre-fix commit (data_column_reconstruction_at_deadline race) — fix in HEAD, tonight's nightly will pass
- 0 compatible dep updates. No code changes needed

### run 884 (Mar 10)
- Spec scan: no new consensus-specs commits since run 883. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- Recently merged PRs: unchanged (latest: #4995 Python 3.14 support — CI/tooling only)
- CI green. No code changes needed

### run 883 (Mar 10)
- Spec scan: no new consensus-specs commits since run 874. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- Recently merged PRs: unchanged (latest: #4995 Python 3.14 support — CI/tooling only)
- CI green on latest code commit. 0 compatible dep updates. No code changes needed

### run 874 (Mar 10)
- Spec scan: no new consensus-specs commits since run 873. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- Recently merged PRs: unchanged (latest: #4995 Python 3.14 support — CI/tooling only)
- CI green (run 22905618379 in progress on latest push). 0 compatible dep updates. No code changes needed

### run 871 (Mar 10)
- Spec scan: no new consensus-specs commits since run 870. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- CI green. Updated Cargo.lock (windows-sys 0.61.2 transitive deps — 10 crates consolidated to latest windows-sys)

### run 870 (Mar 10)
- Spec scan: no new consensus-specs commits since run 869. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked, no approvals)
- CI green. Nightly Mar 10 failure was `network-tests (fulu)` on pre-fix commit — fix already merged. 0 compatible dep updates. No code changes needed

### run 869 (Mar 10)
- Spec scan: no new consensus-specs commits since run 868. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked, updated 2026-03-10T12:05:25Z)
- CI green. Nightly Mar 10 failure confirmed on pre-fix commit (ran 09:04 UTC, fix pushed 09:59 UTC) — tonight's nightly should pass
- 0 compatible dep updates. cargo audit: same rsa advisory (no fix). No code changes needed

### run 868 (Mar 10)
- Spec scan: no new consensus-specs commits since run 867. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- Recently merged PRs: unchanged (latest: #4995 Python 3.14 support — CI/tooling only)
- CI green. 0 compatible dep updates. No code changes needed

### run 867 (Mar 10)
- Spec scan: no new consensus-specs commits since run 866. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- Recently merged PRs: unchanged (latest: #4995 Python 3.14 support — CI/tooling only)
- CI green. 0 compatible dep updates. No code changes needed

### run 866 (Mar 10)
- Spec scan: no new consensus-specs commits since run 865. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- Recently merged PRs since last check: none new (latest: #4995 Python 3.14 support, #4994 test framework cleanup — both CI/tooling)
- CI green (run 22901292778). Nightly Mar 10 failure was network-tests(fulu) on pre-fix commit; fix 62df5686 on main, tonight's nightly should pass
- 0 compatible dep updates. No code changes needed

### run 865 (Mar 10)
- Spec scan: no new consensus-specs commits since run 864. All 11 tracked PRs still OPEN. No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked). No new code commits since potuz's Mar 10 01:05 UTC updates
- Notable merged PRs since last check: #4984 (remove EIP-6800 Verkle specs) — not relevant to Gloas
- CI green. Nightly Mar 10 failure confirmed on pre-fix commit 0d12a857; fix 62df56862 verified stable (5/5 local passes)
- cargo check: zero warnings. No compatible dep updates. cargo audit: same rsa advisory (no fix)
- No code changes needed

### run 859 (Mar 10)
- Spec scan: no new consensus-specs commits since run 858 (latest: #4995 Python 3.14 support). No new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked). potuz addressed review comments (Mar 10 01:05-01:07 UTC) but no new code commits
- CI green. Nightly Mar 10 failure still on pre-fix commit 0d12a857; fix on main, tonight's nightly should pass
- cargo check: zero warnings. No compatible dep updates. cargo audit: same rsa advisory (no fix)
- No code changes needed

### run 857 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- CI green (run 22901292778). Nightly Mar 10 failure on pre-fix commit 0d12a857 (network-tests fulu); fix 62df5686 already on main
- No compatible dep updates. No code changes needed

### run 853 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954, #4898, #4892, #4843, #4939), no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- Recent consensus-specs commits: unchanged — all CI/tooling, no consensus changes
- CI green (run 22901292778, all 6 jobs passed); nightly Mar 10 failure confirmed as pre-fix commit (data_column_reconstruction_at_deadline race), fix 62df5686 on main
- EF spec tests: 35/35 passed (minimal, fake_crypto)
- cargo check: zero warnings; no compatible dep updates
- No code changes needed

### run 851 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954, #4898, #4892, #4843, #4939), no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked)
- Recent consensus-specs commits: unchanged — all CI/tooling, no consensus changes
- CI run 22901292778 (workflow_dispatch commit): all 6 jobs in progress, retrying after previous transient 403
- cargo check: zero warnings
- No code changes needed

### run 850 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954, #4898, #4892, #4843, #4939), no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- Recent consensus-specs commits: all infrastructure/tooling (Python 3.14, test framework, renovate migration, dep bumps) — no consensus changes
- CI run 22900909592: clippy failed due to transient 403 fetching quick-protobuf git dep (not a code issue); other jobs still running
- Added workflow_dispatch trigger to ci.yml for manual re-runs
- No consensus code changes needed

### run 882 (Mar 10)
- Spec scan: all 11 PRs still OPEN, no new merges, no new release (still v1.7.0-alpha.2), no new spec-test vectors
- PR #4992 (PTC lookbehind): unchanged (215962a9, blocked, no approvals)
- CI run 22905618379: 4/6 green, beacon_chain/http_api/unit still running
- No code changes needed

### runs 845-849 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954, #4898, #4892, #4843, #4939), no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PR #4992 (PTC lookbehind): unchanged (head 215962a9, mergeable=blocked, 10 review comments)
- CI green; no new consensus-specs commits with consensus changes
- No code changes needed

### run 844 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954, #4898, #4892, #4843, #4939), no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- Recent consensus-specs commits: reference test generation framework overhaul (#4994, #4993, #4991), removed EIP-6800 Verkle specs (#4984), Python 3.14 support (#4995) — no consensus changes
- v1.6.1 spec test vectors exist but are pre-Gloas (Nov 2025); v1.7.0-alpha.2 remains latest with Gloas tests
- cargo check + clippy: zero warnings; CI green
- Nightly Mar 10 failure confirmed on pre-fix commit 0d12a857; fix (62df5686) already on main, tonight's nightly will pick it up
- No code changes needed

### run 841 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954, #4898, #4892, #4843, #4939), no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Only new consensus-specs commit since run 840: #4995 (Python 3.14 support) — CI/tooling, no consensus changes
- cargo check + clippy: zero warnings; CI green
- EF spec tests: 35/35 passed (minimal, fake_crypto)
- cargo audit: 1 advisory (rsa, no fix); cargo outdated: only major bumps, no compatible patches
- Nightly Mar 10 failure confirmed on pre-fix commit 0d12a857; fix (62df5686) already on main
- No code changes needed

### run 836 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954, #4898, #4892, #4843, #4939), no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PR #4992: unchanged (head 215962a9, mergeable_state=blocked, 10 review comments)
- Latest consensus-specs commits: unchanged — all CI/tooling, no consensus changes
- cargo check + clippy: zero warnings; CI green
- EF spec tests: 35/35 passed (minimal, fake_crypto)
- cargo audit: 1 advisory (rsa, no fix); cargo outdated: only major bumps, no compatible patches
- Nightly failure (Mar 10 09:04) was on pre-fix commit; tonight's nightly will run fix commit 62df5686
- No code changes needed

### run 835 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954, #4898, #4892, #4843, #4939), no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- #4992 (PTC lookbehind minimal cache) most active — potuz + kevaundray reviewing, community prefers over #4979
- Latest consensus-specs merges (Mar 7-10): all CI/tooling (#4995, #4994, #4993, #4991, #4990, #4988), no consensus changes
- cargo check + clippy: zero warnings; CI green
- EF spec tests: 35/35 passed (minimal, fake_crypto)
- cargo audit: 1 advisory (rsa, no fix); cargo outdated: only major bumps, no compatible patches
- Nightly failure (Mar 10 09:04) confirmed on pre-fix commit 0d12a857; fix (62df5686) already in CI green run
- No code changes needed

### run 834 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Latest consensus-specs commits: unchanged — all CI/tooling, no consensus changes
- cargo check + clippy: zero warnings; CI green (run 22897038351, all 6 jobs passed)
- EF spec tests: 35/35 passed (minimal, fake_crypto)
- cargo audit: 1 advisory (rsa, no fix); cargo outdated: only major bumps (rand 0.8→0.9), no compatible patches
- Codebase TODO audit: 39 TODOs total, most inherited/spec-dependent/low-priority; no actionable gaps found
- Test coverage audit: gloas.rs has 208 unit tests across 9216 lines — comprehensive coverage
- Nightly failure (09:04 UTC) was on pre-fix commit; fix already green in CI
- No code changes needed

### run 823 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Latest consensus-specs commits: unchanged — all CI/tooling, no consensus changes
- PTC lookbehind #4992: still OPEN (mergeable_state=blocked), unchanged since Mar 10 01:07 UTC
- cargo check: zero warnings; EF spec tests: 35/35 passed (minimal, fake_crypto)
- Workspace tests: 2643/2651 passed (8 web3signer_tests timeout — external service)
- cargo audit: 1 advisory (RUSTSEC-2023-0071 rsa, no fix available); no lockfile updates available
- No code changes needed

### run 822 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Latest consensus-specs commits: unchanged — all CI/tooling, no consensus changes
- PTC lookbehind #4992: still OPEN (mergeable_state=blocked), unchanged since Mar 10 01:07 UTC
- cargo check + clippy: zero warnings; CI in progress
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal, fake_crypto)
- Workspace tests: 2643/2651 passed (8 web3signer_tests timeout — external service)
- cargo audit: 1 advisory (RUSTSEC-2023-0071 rsa, no fix available); no lockfile updates available
- No code changes needed

### run 807 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Latest consensus-specs commits: unchanged from run 806 — all CI/tooling, no consensus changes
- PTC lookbehind #4992: still OPEN and MERGEABLE, unchanged since Mar 10 01:07 UTC
- cargo check + clippy: zero warnings; CI in progress
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal, fake_crypto)
- cargo audit: 1 advisory (RUSTSEC-2023-0071 rsa, no fix available, low practical risk); cargo outdated: 3 minor (rand/rand_xorshift/rand_chacha), none actionable
- No code changes needed

### run 806 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Latest consensus-specs commits: python 3.14 (#4995), reftest cleanup (#4994, #4993, #4991), release-drafter update (#4990) — all CI/tooling, no consensus changes
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC (head: 215962a9), 2 commits, still awaiting approvals
- cargo check: zero warnings; CI in progress (latest green: Mar 9)
- EF spec tests: 35/35 passed (minimal, fake_crypto)
- No compatible dependency updates available
- No code changes needed

### run 805 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954, #4898, #4892, #4843, #4940, #4932, #4960, #4962), no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Latest consensus-specs commits: EIP-6800 removal (#4984), renovate/release-drafter updates — no Gloas changes
- cargo check + clippy: zero warnings; CI in progress (latest green: Mar 9)
- EF spec tests: 35/35 passed (minimal, fake_crypto)
- Workspace tests: 2643/2652 passed (9 web3signer_tests timeout — external service)
- cargo audit: 1 vuln (unchanged), 5 allowed warnings; no lockfile updates available
- No code changes needed

### run 800 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954), no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Latest consensus-specs commits: python 3.14 support (#4995), reftest cleanup (#4994) — CI/tooling only, no new commits since last run
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, design stable but blocked on approvals
- Analyzed PTC lookbehind implementation plan: 7 code areas to change when PR merges (BeaconState field, compute_ptc extraction, get_ptc cache read, process_slots shift, fork upgrade init, fork choice reorder, validator duties)
- cargo check + clippy: zero warnings; CI green; nightly: 5 consecutive green (Mar 5-9); cargo audit: no new advisories; no lockfile updates
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- No code changes needed

### run 799 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954), no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Latest consensus-specs commits: python 3.14 support (#4995), reftest cleanup (#4994) — CI/tooling only, no new commits since last run
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC
- EF spec tests: 35/35 passed; fork choice EF tests: 8/8 passed; cargo check: zero warnings
- No code changes needed

### run 798 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954), no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Latest consensus-specs commits: python 3.14 support (#4995), reftest cleanup (#4994) — CI/tooling only, no new commits since last run
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC
- EF spec tests: 35/35 passed; cargo check: zero warnings; CI green
- No code changes needed

### run 797 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954), no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Latest consensus-specs commits: python 3.14 support (#4995), reftest cleanup (#4994) — CI/tooling only
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC
- EF spec tests: 35/35 passed; fork choice EF tests: 8/8 passed
- cargo check --release: zero warnings; CI green
- No code changes needed

### run 796 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954), no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Latest consensus-specs commits: python 3.14 support (#4995), reftest cleanup (#4994) — CI/tooling only
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, design discussion ongoing (get_ptc_assignment 32-slot vs 2-slot cache unresolved)
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- Fork choice EF tests: 8/8 passed (real crypto)
- cargo check --release: zero warnings; CI green
- No code changes needed

### run 795 (Mar 10)
- Spec scan: all tracked PRs still OPEN (#4992, #4979, #4954), no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- cargo check --release: zero warnings; CI green
- No code changes needed

### run 784 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still open
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- cargo check --release: zero warnings; CI green
- No code changes needed

### run 779 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still open
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- cargo check --release: zero warnings; CI green
- No code changes needed

### run 771 (Mar 10)
- Spec scan: no new Gloas merges (recent: python 3.14 support, reftest cleanup — CI/tooling only), no new spec release (still v1.7.0-alpha.2)
- All tracked PRs still OPEN; PTC lookbehind #4992 unchanged since Mar 9
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- cargo check --release: zero warnings; cargo audit: rsa advisory only; CI green; nightly: 3 consecutive green (Mar 7-9)
- Docker workflow queued ~12h (self-hosted runner unavailability, not code)
- No code changes needed

### run 761 (Mar 10)
- Spec scan: no new Gloas merges (recent: python 3.14 support — CI/tooling only), no new spec release (still v1.7.0-alpha.2)
- All tracked PRs still OPEN; PTC lookbehind #4992 unchanged since Mar 9
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- cargo check --release: zero warnings; cargo audit: rsa advisory only; CI green
- No code changes needed

### run 760 (Mar 10)
- Spec scan: no new Gloas merges (recent: python 3.14 support — CI/tooling only), no new spec release (still v1.7.0-alpha.2)
- All 7 tracked PRs still OPEN; PTC lookbehind #4992 got 1 positive review (ensi321 prefers it over #4979)
- New untracked PR #4962: sanity/blocks tests for missed payload withdrawal interactions (test-only, no spec change)
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- cargo check --release: zero warnings; cargo audit: rsa advisory only; nightly CI: 3 consecutive green
- No code changes needed

### run 759 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges (recent: python 3.14 support, reftest cleanup — all CI/tooling), no new spec release (still v1.7.0-alpha.2)
- No new activity on PTC lookbehind PRs (#4992/#4979) since Mar 9
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- Clippy clean; cargo audit unchanged (rsa advisory only); no compatible dependency updates
- CI green (docker still queued — runner availability, not code)
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 744)
- Spec scan: no new Gloas merges (recent: reftests, release-drafter, nightly matrix, deps cleanup — all CI/tooling), no new release (still v1.7.0-alpha.2), all 10 tracked PRs still OPEN
- PTC lookbehind PR #4992: potuz updated today — acknowledged duties functions not yet touched, awaiting dev preferences on approach vs #4979
- CI (22875542079): all 6 jobs passed; nightly: 5 consecutive green runs (Mar 5-9)
- cargo check --release: zero warnings; cargo audit: 1 known rsa advisory, 5 allowed warnings, no new vulnerabilities
- cargo outdated: rand_xorshift 0.4→0.5 available (non-critical), rand 0.8→0.9 (dev dep only)
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 741)
- Spec scan: no new Gloas merges (recent merges all CI/tooling/deps), no new release (still v1.7.0-alpha.2), all 10 tracked PRs still OPEN
- CI (22875542079): all 6 jobs passed — check+clippy+fmt, EF tests, network+op_pool, http_api, beacon_chain, unit tests
- cargo check --release: zero warnings; cargo audit: 1 known rsa advisory, 5 allowed warnings, no new vulnerabilities
- Dependencies: rand_xorshift has minor update (0.4→0.5), not critical
- Unwrap audit: consensus production code clean — only safe unwraps on type-bounded collections, zero in fork_choice/state_processing production paths
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 740)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), all 10 tracked PRs still OPEN
- CI: check+clippy+fmt passed, EF tests passed, network+op_pool passed, http_api passed, beacon_chain/unit still running
- cargo check --release: zero warnings; cargo audit: 1 known rsa advisory, 5 allowed warnings, no new vulnerabilities
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 739)
- Spec scan: no new Gloas merges (3 merged: reftests, nightly matrix, release-drafter — all CI/tooling), no new release (still v1.7.0-alpha.2), all 10 tracked PRs still OPEN
- PTC lookbehind PR #4992: active review today — kevaundray raised that 2-slot cache doesn't cover `get_ptc_assignment`'s 32-slot iteration; potuz acknowledges duties functions not yet addressed, design still evolving
- CI (22875542079): check+clippy+fmt passed, EF tests passed, network+op_pool passed, http_api passed, beacon_chain/unit still running
- cargo check --release: zero warnings; cargo audit: 1 known rsa advisory, 5 allowed warnings, no new vulnerabilities
- Dependencies: 0 compatible crate updates available
- Test coverage audit: load_parent patching, epoch processing, self-build envelope all thoroughly covered; no actionable gaps found
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 738)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), all 10 tracked PRs still OPEN
- CI (22875542079): check+clippy+fmt passed, EF tests passed, network+op_pool passed, beacon_chain/unit/http_api in progress
- cargo check --release: zero warnings; cargo audit: 1 known rsa advisory, 5 allowed warnings, no new vulnerabilities
- Dependencies: 0 compatible crate updates available
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 737)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), all 10 tracked PRs still OPEN
- Checked 2 untracked PRs: #4840 (EIP-7843, stale since Jan, 0 comments) and #4630 (EIP-7688 forward-compat SSZ, stale since Feb) — not worth tracking yet
- CI (22875542079): check+clippy+fmt passed, EF tests passed, remaining jobs in progress
- cargo check --release: zero warnings; cargo audit: 1 known rsa advisory, 5 allowed warnings, no new vulnerabilities
- Dependencies: 0 compatible crate updates available
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 736)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), all 10 tracked PRs still OPEN
- CI (22875542079): check+clippy+fmt passed, EF tests passed, remaining jobs in progress; nightly: continuously green
- cargo check --release: zero warnings; cargo audit: 1 known rsa advisory (RUSTSEC-2023-0071), 5 allowed warnings, no new vulnerabilities
- Test coverage audit: process_pending_envelope paths fully covered (happy, EL Syncing, EL Invalid, re-verification failures)
- No code changes needed

### 2026-03-09 — fixed 2 beacon_chain test failures (run 735)
- Found and fixed 2 tests that fail when running `cargo nextest run -p beacon_chain` without feature flags:
  - `block_verification::verify_block_for_gossip_slashing_detection`: panicked on `Slasher::open().unwrap()` when no slasher backend compiled in. Fixed: gracefully skip when `SlasherDatabaseBackendDisabled`.
  - `column_verification::rpc_columns_with_invalid_header_signature`: guard condition checked `is_fulu_scheduled()` (true on mainnet default spec, Fulu at epoch 411392) instead of checking genesis fork. With phase0 genesis, block production has no blobs → `opt_blobs.unwrap()` panicked. Fixed: check `fork_name_at_epoch(0).fulu_enabled()` instead.
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), all 10 tracked PRs still OPEN
- CI: all jobs green, nightly: continuously green
- cargo check --release: zero warnings; cargo audit: same known rsa advisory
- 768/768 beacon_chain tests pass (was 766/768 before fix)

### 2026-03-09 — spec stable, all clear (run 734)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0; spec-tests repo archived)
- All 10 tracked PRs still OPEN: #4992, #4979, #4954, #4898, #4892, #4843, #4940, #4932, #4960, #4962
- CI (22871434411): all jobs passed; nightly: continuously green
- cargo check --release: zero warnings; cargo audit: same known rsa advisory, no new vulnerabilities
- Dependencies: 0 compatible updates (all at latest)
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 733)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Recent consensus-specs merges: all CI/tooling/deps (EIP-6800 removal, EIP-7441 removal, action improvements, dep updates)
- Tracked PRs all still OPEN: #4992 (PTC lookbehind minimal, updated today), #4979 (PTC lookbehind cache), #4954 (fork choice milliseconds)
- CI (22871434411): all 7 jobs passed (including ci-success gate); nightly: 5+ consecutive green (Mar 5-9)
- cargo check --release: zero warnings; cargo audit: same known rsa advisory, no new vulnerabilities
- Full test coverage audit: no gaps found across 709+ Gloas unit tests + 306 integration tests
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 732)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Recent consensus-specs merges: all CI/tooling/deps (reftests, nightly matrix, sampling config, dep cleanups, EIP-6800 removal)
- Tracked PRs all still OPEN: #4992 (PTC lookbehind minimal), #4979 (PTC lookbehind cache), #4954 (fork choice milliseconds)
- CI (22871434411): all 6 jobs passed; nightly: 7+ consecutive green (Mar 4-9)
- cargo check --release: zero warnings; cargo audit: same known rsa advisory, no new vulnerabilities
- Deep code quality audit: no unwrap() in consensus paths, no unsafe blocks, all production code well-tested
- Investigated JustifiedBalances clone in find_head hot path (proto_array_fork_choice.rs:611,633) — would require Arc refactor across ForkChoiceStore trait boundary, not worth the risk for unmeasured perf concern
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 731)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Recent consensus-specs merges: all CI/tooling/deps (reftests improvements, renovate, EIP-6800 removal, dep cleanups)
- PTC lookbehind PR #4992: active today — potuz/kevaundray still discussing `get_ptc_assignment` scope (32 slots vs 2-slot cache); potuz hasn't addressed duties functions, waiting on dev preference
- Fast Confirmation Rule PR #4747: updated today, still in review (eip7805/FOCIL scope, Heze fork — not Gloas)
- CI (22871434411): all 6 jobs passed; nightly: 7+ consecutive green (Mar 4-9)
- cargo audit: same known rsa advisory, no new vulnerabilities; 0 compatible dep updates
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 743)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.5.0)
- All 10 tracked PRs still OPEN; PTC lookbehind (#4992) still in design (potuz: duties functions not yet addressed)
- CI: all 6 jobs passed; nightly: 3+ consecutive green
- Local test verification: 575/575 state_processing, 302/302 fork_choice+proto_array — all pass
- cargo check --release: zero warnings; cargo audit: no new vulnerabilities
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 730)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- Recent consensus-specs merges: all CI/tooling/deps (pytest perf, reftests, renovate, EIP-6800 removal)
- PTC lookbehind PRs (#4979, #4992) still OPEN; potuz confirmed duties functions not yet addressed, waiting on dev preference
- CI (22871434411): all 6 jobs passed (check+clippy+fmt, EF tests, network+op_pool, http_api, beacon_chain, unit tests)
- Nightly: 6+ consecutive green (Mar 4-9)
- cargo check --release: zero warnings, 0 compatible dep updates
- cargo audit: same known rsa advisory, no new vulnerabilities
- Full codebase coverage audit: no todo!()/unimplemented!() in production, no unwrap() in consensus paths, no unsafe blocks, no #[allow(dead_code)] on Gloas code
- No code changes needed

### 2026-03-09 — spec stable, safety audit clean (run 729)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0)
- PTC lookbehind PRs (#4979, #4992) still OPEN, design not finalized
- CI (22871434411): 5/6 passed (check+clippy+fmt, EF tests, network+op_pool, http_api, beacon_chain), unit tests still running
- Nightly: 6+ consecutive green (Mar 4-9)
- cargo check --release: zero warnings
- cargo update --dry-run: 0 compatible updates available
- Production code safety audit (gloas.rs, fork_choice.rs, proto_array.rs, block_verification.rs, beacon_chain.rs): no unwrap() in consensus paths, all array access via .get(), saturating/checked arithmetic throughout, no unsafe blocks
- Gossip verification test coverage audit: 42+ tests covering all happy paths; remaining untested error variants (InvalidAggregationBits, PtcCommitteeError, MissingBeaconBlock, NotGloasBlock) are hard-to-reach paths requiring malformed internal state — covered implicitly by integration tests
- No code changes needed

### 2026-03-09 — spec stable, all clear (run 728)
- Spec scan: no new Gloas merges, no new release (still v1.7.0-alpha.2)
- PTC lookbehind PR #4992 (minimal approach): active review comments from potuz/kevaundray, `get_ptc_assignment` needs fixing for 2-slot cache vs 32-slot iteration — design not finalized
- CI (22871434411): 5/6 passed (check+clippy+fmt, EF tests, network+op_pool, http_api), unit+beacon_chain still running
- Nightly: 6 consecutive green (Mar 4-9)
- cargo audit: same known rsa advisory, no new vulnerabilities
- Deep audit of `process_builder_pending_payments`, `upgrade_to_gloas`, `process_proposer_lookahead`, `load_parent` Gloas patching: all correct, well-tested
- Epoch processing ordering verified: pending_consolidations → builder_pending_payments → effective_balance_updates → proposer_lookahead (matches spec)
- No code changes needed

### 2026-03-09 — spec stable, routine audit (run 727)
- Spec scan: no new Gloas merges (last 10 merged PRs all CI/tooling/deps), no new release (still v1.7.0-alpha.2)
- PTC lookbehind PRs (#4979, #4992) still open; Fast Confirmation Rule PR #4747 updated today
- CI (22871434411): 3/6 passed (check+clippy+fmt, EF tests, network+op_pool), 3 still running (unit, beacon_chain, http_api)
- Nightly: 6 consecutive green (Mar 4-9), all 26 jobs passing
- cargo audit: same 1 vulnerability (rsa RUSTSEC-2023-0071, no fix), 5 unmaintained warnings (ansi_term, bincode, derivative, filesystem, paste — all transitive, not actionable)
- Dependencies: 0 compatible updates, 30 major-version bumps available (not actionable)
- Full Gloas test coverage audit: 90%+ across all production code, no critical gaps (observed collections, bid pool, gossip handlers, HTTP API, state processing all well-tested)
- Devnet verification launched (--no-build), results pending
- No code changes needed

### 2026-03-09 — spec stable, comprehensive coverage audit (run 726)
- Spec scan: all 10 tracked PRs still OPEN, no new Gloas merges, no new release (still v1.7.0-alpha.2)
- No new spec-test vectors (consensus-spec-tests latest: v1.6.0-beta.0, pre-Gloas)
- PTC lookbehind PR #4992: still evolving — potuz acknowledged duties functions not yet addressed, kevaundray raised get_ptc_assignment needing 32 slots vs 2-slot cache
- CI in progress (22871434411): check+clippy+fmt passed, remaining jobs running; nightly green 5 consecutive runs (Mar 5-9)
- cargo audit: same known rsa advisory (RUSTSEC-2023-0071), no new vulnerabilities
- Dependencies: all at latest compatible versions (0 crate updates available)
- Deep test coverage audit across consensus crates:
  - state_processing/per_block_processing/gloas.rs: 9200+ lines including comprehensive tests for all Gloas functions (bid processing, envelope processing, withdrawals, PTC committee, builder exits, deposit routing)
  - fork_choice.rs: 80 Gloas-specific tests covering on_execution_bid, on_payload_attestation, on_execution_payload, validate_on_attestation, gloas_head_payload_status, queued attestation dequeue
  - proto_array: 300+ tests including propagation, viability, find_head_gloas, contains_execution_block_hash
  - upgrade/gloas.rs: 21+ tests covering state migration, builder onboarding, field preservation
  - execution_bid_pool.rs: 17 tests covering bid selection, pruning, reorg, parent filtering
  - observed_*.rs: 45+ tests for equivocation tracking, deduplication, pruning
- Only gap: gloas_verification.rs (gossip verification) has no unit tests — covered by integration tests in network_beacon_processor/tests.rs, which is appropriate for BeaconChain-dependent code
- Zero todo!()/unimplemented!() in production code, clippy clean
- No code changes needed

### 2026-03-09 — contains_invalid_payloads + on_invalid_execution_payload tests (run 725)
- Spec scan: all 10 tracked PRs still OPEN, no new Gloas merges, no new release (still v1.7.0-alpha.2)
- PTC lookbehind PR #4992: potuz acknowledged `get_ptc_assignment` needs fixing, waiting on dev preference for approach
- Added 4 unit tests for `contains_invalid_payloads` (ProtoArrayForkChoice): empty tree, all valid, after invalidation, mixed valid/invalid
- Added 3 unit tests for `on_invalid_execution_payload` (ForkChoice): InvalidateOne transitions Optimistic→Invalid, unknown root error, InvalidateMany with known ancestor
- Both functions previously had no dedicated test coverage despite being part of the execution validity pipeline
- All 302 proto_array + fork_choice tests pass, clippy clean

### 2026-03-09 — spec stable, codebase audit (run 724)
- Spec scan: all 10 tracked PRs still OPEN, no new Gloas merges, no new release (still v1.7.0-alpha.2)
- No new spec-test vectors (consensus-spec-tests latest: v1.6.0-beta.0, pre-Gloas)
- CI running on latest commit (22870841207), nightly green 5 consecutive runs (Mar 5-9)
- cargo audit: same known rsa advisory, no new vulnerabilities
- Codebase audit: zero `todo!()`/`unimplemented!()` in production code (64 instances all in test mock `ValidatorStore` trait impls, acceptable)
- Test coverage audit: all major Gloas functions comprehensively tested (150+ state_processing, 50+ proto_array, 40+ envelope, 16+ epoch processing, integration tests)
- No code changes needed

### 2026-03-09 — proto_array propagation tests (run 723)
- Spec scan: all 10 tracked PRs still OPEN, no new Gloas merges, no new release (still v1.7.0-alpha.2)
- PTC lookbehind PR #4992: design still evolving, not ready to implement
- Added 15 unit tests for consensus-critical proto_array functions that had no test coverage:
  - `propagate_execution_payload_validation` (6 tests): ancestor chain propagation, stops at Valid/Irrelevant, errors on Invalid ancestor, unknown root, single node
  - `propagate_execution_payload_invalidation` (6 tests): InvalidateOne target+descendants, best_child clearing, unknown root, InvalidateMany with known/unknown ancestor, head-only skipping
  - `node_leads_to_viable_head` (3 tests): via best_descendant, self-viable, invalid descendant with unrevealed payload
- All 179 proto_array tests pass, clippy clean

### 2026-03-09 — spec stable, deep conformance audit (run 718)
- No new Gloas merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors (v1.5.0)
- All tracked PRs still OPEN; #4992 (PTC Lookbehind minimal) most active with review from potuz/kevaundray
- CI green, nightly green (3 consecutive)
- Deep spec conformance audit of all Gloas functions against consensus-specs master:
  - `compute_balance_weighted_selection` — verified: `compute_proposer_index`, `get_next_sync_committee_indices`, `get_ptc_committee` all correctly implement the selection algorithm (shuffle_indices=True for proposer/sync, False for PTC)
  - `compute_balance_weighted_acceptance` — verified: 16-bit random, LE bytes, correct hash preimage (seed + i/16)
  - `process_execution_payload` (envelope) — verified: all 12 verification checks match spec, builder payment queuing correct
  - `process_builder_pending_payments` — verified: quorum check, window rotation, ordering within epoch processing
  - `process_slot` — verified: availability clearing at `(slot+1) % SLOTS_PER_HISTORICAL_ROOT`
  - `process_epoch` — verified: `builder_pending_payments` called after `pending_consolidations`, before `effective_balance_updates`
- No code changes needed — all implementations match spec

### 2026-03-09 — spec stable, test coverage expanded (run 717)
- No new Gloas merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 10 tracked PRs still OPEN
- CI green (ci success)
- Added 6 unit tests for `process_payload_attestation` and `get_indexed_payload_attestation` — happy path (single/all bits), empty bits rejection, and index conversion correctness
- These consensus-critical functions previously only had error-path tests for root/slot validation

### 2026-03-09 — spec stable, no changes (run 715)
- No new Gloas merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 10 tracked PRs still OPEN; #4992 (PTC Lookbehind minimal) most actively reviewed
- CI green (ci success), docker queued (runner availability), zero compiler warnings
- Cargo audit unchanged (1 vuln RUSTSEC-2023-0071 rsa, 5 allowed warnings — all transitive/no-fix)
- Dependencies all at latest compatible versions (0 updates)
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 714)
- No new Gloas merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 10 tracked PRs still OPEN, no new activity
- CI green (ci success), docker queued (runner issue), zero compiler warnings
- Dependencies all at latest compatible versions (0 updates)
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 710)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 10 tracked Gloas PRs still OPEN; #4992 (PTC Lookbehind) most active with 4 review comments
- CI fully green (all 6 jobs passed on latest commit), cargo audit unchanged (1 rsa advisory, transitive)
- Dependencies all at latest compatible versions (cargo update --dry-run shows 0 updates)
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 709)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 10 tracked Gloas PRs still OPEN; no new merges since last check
- CI green, clippy clean (zero warnings), cargo audit unchanged (1 rsa advisory, transitive)
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 707)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- Recent merges: #4993 (reftests), #4990/#4991 (CI), #4988 (sampling config), #4984 (removed Verkle/EIP-6800) — none Gloas-related
- All 10 tracked Gloas PRs still OPEN; #4992 (PTC Lookbehind minimal) actively reviewed
- CI run in progress: 3/6 complete (ef-tests, network+op_pool, check+clippy+fmt all green), 3 running
- Nightly CI: 3 consecutive green runs (Mar 7-9)
- Clippy clean (zero warnings), cargo audit unchanged (1 rsa advisory, transitive), 0 dep version changes
- Test coverage audit: all major Gloas modules well covered (55 envelope processing, 23 builder payments, 80+ fork choice, 306+ beacon_chain integration tests)
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 706)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 10 tracked Gloas PRs still OPEN; PR #4992 getting review from kevaundray
- CI: ef-tests/network+op_pool/check+clippy+fmt passed; 3 jobs in progress
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 705)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 10 tracked Gloas PRs still OPEN: #4992, #4979, #4960, #4954, #4940, #4939, #4932, #4898, #4892, #4843
- CI: last completed run success, current run in progress
- Clippy clean (zero warnings), cargo audit unchanged (1 medium rsa advisory, transitive)
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 703)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 4 tracked Gloas PRs still OPEN: #4992, #4979, #4954, #4843
- No new Gloas PRs opened since last scan
- CI run from previous commit still in progress
- Clippy clean (zero warnings)
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 702)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 4 tracked Gloas PRs still OPEN: #4992, #4979, #4954, #4843
- CI run from previous commit still in progress (all 6 jobs running)
- Clippy clean (zero warnings), cargo audit unchanged (1 medium rsa advisory, transitive)
- No code changes needed

### 2026-03-09 — implemented PR #4939, spec scan (run 701)
- PR #4939 (index-1 attestation envelope validation) MERGED Mar 7 — implemented gossip-level checks
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test vectors
- Still tracked OPEN: #4992, #4979, #4954, #4843
- New Gloas PRs: EIP-7866 Inclusion Lists cluster (7 PRs), #5009 PTC entries, #5012 execution request gossip, #5014 networking fixes
- All tests pass, clippy clean

### 2026-03-09 — spec stable, no changes (run 700)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 12 tracked Gloas PRs still OPEN — no activity since last scan
- CI run 690 in progress (3/6 jobs complete, 3 running), run 680 green
- Clippy clean (zero warnings), cargo audit unchanged (1 medium rsa advisory, transitive), 0 dep updates
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 698)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All tracked Gloas PRs still OPEN — #4992 (PTC Lookbehind minimal) still blocked, no new reviews
- CI green (last completed run success), clippy clean (zero warnings)
- cargo audit unchanged (1 medium rsa advisory, transitive), 0 dep updates
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 695)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- 8 tracked Gloas PRs still OPEN (#4992, #4979, #4960, #4940, #4932, #4843, #4840, #4630)
- Recently merged: #4991 CI matrix, #4990 release-drafter — maintenance only
- CI: last run green, current run in progress
- cargo audit unchanged, 0 dep updates available
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 694)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 13 tracked Gloas PRs still OPEN — no new reviews or approvals
- PR #4992 (PTC Lookbehind minimal): potuz self-review comments on naming/placement, still blocked
- CI in progress (run 690), last completed run green
- cargo audit unchanged (1 medium rsa advisory, transitive), 0 dep updates available
- No code changes needed

### 2026-03-09 — spec stable, no changes (run 693)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- All 13 tracked Gloas PRs still OPEN — #4843, #4898, #4892 still approved but not merged
- CI green, nightly 5+ consecutive greens, 0 dep updates, cargo audit unchanged
- No code changes needed

### 2026-03-09 — spec stable, approved PRs tracked (run 692)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- 3 approved PRs ready to merge: #4843 (Variable PTC deadline — significant timing change), #4898 (remove pending status from tiebreaker — cleanup), #4892 (remove impossible branch in forkchoice — cleanup)
- Previously merged PRs #4918 (attestation payload status check) and #4947 (pre-fork proposer_preferences subscription) both already implemented in vibehouse
- CI green (run 680 success, current run 690 in progress), nightly 5+ consecutive greens
- Zero patch dep updates available, cargo audit unchanged (1 medium rsa advisory, transitive)
- Code audit: builder withdrawal apply path (gloas.rs:686) uses defensive `get_mut()` with `if let Some` — safe because builder_index is validated at line 516-520 during withdrawal creation
- No code changes needed — monitoring #4843 for merge (would need PTC deadline timing implementation)

### 2026-03-09 — spec stable, codebase audit (run 684)
- Spec stable: no new merges, no new releases (v1.7.0-alpha.2), no new spec-test vectors
- 11 open Gloas PRs tracked (#4992, #4979, #4960, #4954, #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4747, #4558)
- PR #4992 (PTC Lookbehind minimal): 2 review comments from potuz (naming, placement), still blocked/open
- CI run: check+clippy ✓, ef-tests ✓, network+op_pool ✓, http_api ✓, beacon_chain+unit tests in progress
- Zero clippy warnings, zero TODOs in production code, zero unwraps in consensus production code
- cargo audit: same 1 medium rsa advisory (transitive, no fix available)
- Dependency check: no actionable updates (only dev dep `rand` semver-major, `rand_xorshift` major)
- Test coverage audit: gloas_verification.rs has 60 tests, single_pass.rs has 24 Gloas tests, builder pending payments has 19 tests, block_replayer has 32 tests — all critical paths well covered
- No code changes needed

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
