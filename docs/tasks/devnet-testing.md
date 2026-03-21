# Devnet Testing

## Objective

Test vibehouse under diverse devnet scenarios beyond the happy path. The initial 4-node devnet (priority 1) proved basic functionality — this task covers syncing, node churn, long-running, and adversarial scenarios.

## Status: DONE (all scenarios implemented)

### Scenarios

| Scenario | Status | Detail |
|----------|--------|--------|
| Syncing (genesis sync) | DONE (script) | `--sync` flag: 2 validators + 2 sync targets, nodes catch up through Gloas fork |
| Node churn | DONE (script) | `--churn` flag: kill validator node 4, verify chain continues (75% stake), restart, verify recovery |
| Mainnet preset | DONE (script) | `--mainnet` flag: 4 nodes, 512 validators, 32 slots/epoch, 12s slots, ~40 min timeout |
| Long-running | DONE (script) | `--long` flag: epoch 50 target, periodic memory/CPU monitoring, ~40 min |
| Builder path | DONE (script) | `--builder` flag: genesis builder injection, proposer prefs + bid submission via lcli |
| Payload withholding | DONE (script) | `--withhold` flag: submit bid with no envelope, verify EMPTY path finalization |
| Network partitions | DONE (script) | `--partition` flag: stop 2/4 nodes (50% stake), verify stall, heal, verify finalization resumes |
| Stateless + ZK | DONE | 3 proof-generators + 1 stateless node (from priority 4) |
| Slashing scenarios | DONE (script) | `--slashings` flag: inject double-proposal and double-vote via lcli, verify slashed=true |

## Progress log

### run 2118 (Mar 21) — maintenance check, all stable

CI green (arc-swap update in progress, check+clippy passed). 5 consecutive green nightlies. Spec HEAD unchanged (1baa05e, Mar 15). No new Gloas PRs merged. Zero compiler warnings, zero clippy warnings. cargo audit unchanged (rsa, no fix). No dependency updates available (0 packages to lock). Audited Gloas production code for .unwrap() — all clean, proper Result-based error handling throughout. Open Gloas PRs closest to merging: #4843 (variable PTC deadline, 1 approval, clean mergeable), #4992 (cached PTCs, 25 review comments, clean mergeable). #4747 (fast confirmation) updated today but still dirty/not ready. Project in maintenance mode.

### run 2117 (Mar 21) — dep update, maintenance check

CI green. Spec HEAD unchanged (1baa05e, Mar 15). No new Gloas PRs merged. Updated arc-swap 1.8.2→1.9.0 (execution_layer dep, 145/145 tests pass). Open Gloas PRs: #4747 (fast confirmation, updated Mar 21), #4843 (variable PTC deadline), #4979 (PTC lookbehind) — all still open. cargo audit unchanged. Project in maintenance mode.

### run 2047 (Mar 21) — maintenance check, all stable

CI green. Spec HEAD unchanged (1baa05e, Mar 15). Open Gloas PRs still open — #4843/#4979 updated Mar 20 but not merged. #5008 audited: doc-only field name fix, our code already uses `beacon_block_root` correctly. No dep updates available. cargo check clean. Project in maintenance mode.

### run 2046 (Mar 21) — maintenance check, all green

CI green (last 5 runs). Nightlies green (Mar 18-20). Build/clippy clean, zero warnings. Spec HEAD still 1baa05e from Mar 15 — no new commits. Open Gloas PRs unchanged: #4843 (variable PTC deadline), #4979 (PTC lookbehind), #4992 (cached PTCs), #5020 (PTC lookbehind minimal), #5022 (payload attestation block check), #5023 (block root filenames). None merged. cargo audit: unchanged (rsa no fix, 5 unmaintained transitive deps). Investigated potential perf improvements (state.clone().canonical_root(), Vec allocations in epoch processing) — all either in test code only or already optimized (milhouse CoW makes BeaconState clone cheap). Verified withdrawal edge cases (spec PR #4962) already covered by existing tests. Project in maintenance mode.

### run 2045 (Mar 21) — maintenance check, spec audit, all green

CI green. Nightlies green (3 consecutive green). Build clean, zero warnings. Spec tracked to v1.7.0-alpha.3 — HEAD still 1baa05e from Mar 15, no new commits. Audited 3 new open spec PRs: #5022 (assert block known in on_payload_attestation_message — already handled by our UnknownBeaconBlockRoot error at fork_choice.rs:1426-1432), #5023 (test fixture naming fix + Gloas comptests — test infra only, no code impact until new release), #5020 (PTC lookbehind — still open/draft). PR #5001 (parent_block_root in bid filter key) confirmed already implemented (observed_execution_bids.rs uses 3-tuple since implementation). cargo audit: 1 known (rsa timing, no upstream fix), 5 unmaintained warnings. No dependency updates available. Project in maintenance mode.

### run 1930 (Mar 19) — clippy lint improvements: copied() and filter_map()

Applied two clippy lint improvements across 36 files (66 substitutions): (1) `.cloned()` → `.copied()` for Copy types (62 instances) — `copied()` is more efficient and communicates that the type is Copy, (2) `.flat_map()` → `.filter_map()` for Option return types (5 instances) — `filter_map` is the idiomatic choice. All changes are mechanical and semantically identical. Lint clean, 4991/5000 tests pass (9 web3signer infrastructure-dependent failures, pre-existing). Spec v1.7.0-alpha.3 still latest — no new commits since #5005 (Mar 15). All open Gloas PRs unchanged.

### run 1929 (Mar 19) — spec audit, full EF test verification

CI green. Clippy clean. Audited 4 post-alpha.3 spec PRs: #5001 (parent_block_root bid filter — already compliant), #5002 (wording — no code impact), #4940 (Gloas fork choice tests — all 9 pass including new on_execution_payload), #5005 (test-only fix). Full EF spec test verification: 139/139 (fake_crypto+minimal), 79/79 (real crypto+minimal), 9/9 fork choice tests. Checked open Gloas PRs: #4992 (cached PTCs, 1 approval from jtraglia, active discussion) is most likely next to merge — adds previous_ptc/current_ptc fields to BeaconState. Heze fork research: implements FOCIL (EIP-7805, fork-choice enforced inclusion lists). Project in maintenance mode.

### run 1928 (Mar 19) — maintenance check, all green

CI green. Nightlies green (latest 2 both green). Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 14 tracked Gloas spec PRs still open — none merged. cargo audit: same known issues (rsa, unmaintained transitive deps). cargo-machete false positives investigated (rand, ethereum_ssz all needed by derive macros). No remaining actionable code improvements — all TODOs reference #36 and are blocked or non-critical. Project in maintenance mode.

### run 1919 (Mar 19) — dead code cleanup

Removed 6 unused functions/items across 5 files: `expect_no_work_event` (sync test helper), `get_enr`/`build_linear` (libp2p test helpers), `update_branch` (EE integration build util), `reset_mocks` (validator test rig), `with_config_and_spec`+`chain_config` field (CLI exec tests). Also removed 2 unused imports (`EnrExt`, `Multiaddr`). Full lint-full pass, CI triggered. No new spec changes since #5002 (Mar 13). All 11 open Gloas spec PRs unchanged (none close to merging). Project in maintenance mode.

### run 1918 (Mar 19) — dependency updates, maintenance

Updated patch dependencies: alloy-chains 0.2.31→0.2.32, borsh 1.6.0→1.6.1, toml_edit 0.25.4→0.25.5. Build clean, clippy clean, 1085/1085 types tests pass, 69/69 EF SSZ static tests pass. CI green (previous run). Nightlies green (Mar 17 failure was flake fixed by 8f8faa7de). Spec tracked to v1.7.0-alpha.3 — no new merges. #4992 (cached PTCs) most active open PR (1 approval from jtraglia, still under discussion). #5008 (field name fix) is doc-only, our code already uses correct `beacon_block_root`. All other tracked PRs unchanged. cargo audit: same known issues (rsa, unmaintained sp1 transitive deps). Project in maintenance mode.

### run 1917 (Mar 19) — sp1-verifier version bump, full EF test verification

Updated sp1-verifier workspace dependency specifier from 6.0.1 to 6.0.2 (lockfile already had 6.0.2). Full verification: 139/139 EF tests pass (fake_crypto+minimal), 9/9 fork choice tests pass (real crypto), cargo check clean, clippy clean. No new consensus-specs merges since #5005 (Mar 15). All 14 tracked open Gloas PRs unchanged. cargo audit: 1 error (rsa/jsonwebtoken, no fix), 5 warnings (sp1 transitive deps — ansi_term, bincode, derivative, paste + filesystem false positive on internal crate).

### run 1916 (Mar 19) — maintenance check, all green

CI green. Nightlies green (latest 2 both green, Mar 17 failure was known flake). Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 14 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843, #4954, #4840, #4630, #4558 — none merged. #4992 (cached PTCs) still active (23 reviews, 5 comments). Project in maintenance mode.

### run 1915 (Mar 19) — maintenance check, all green

CI green. Nightlies green (latest 2 both green). Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 14 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843, #4954, #4840, #4630, #4558 — none merged. `cargo audit`: 1 known vulnerability (rsa timing side-channel via jsonwebtoken — no upstream fix). All TODOs in codebase reference issue #36 and are either blocked (EIP-7892, blst, PeerDAS) or non-critical. Project in maintenance mode.

### run 1914 (Mar 19) — maintenance check, discovered 4 untracked spec PRs

CI green. Nightlies green (latest 2 both green, Mar 17 flake predated 8f8faa7d fix). Spec tracked to v1.7.0-alpha.3 (HEAD still 85ab2d2 — no new commits). All 10 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843 — none merged.

Discovered 4 untracked open Gloas spec PRs:
- **#4954** — Update fork choice store to use milliseconds (labels: phase0/bellatrix/gloas/heze, 0 activity, early stage)
- **#4840** — EIP-7843 SLOTNUM opcode for Gloas (0 activity, very early)
- **#4630** — EIP-7688 forward-compatible SSZ types (2 reviews, touches SSZ fundamentals)
- **#4558** — Cell Dissemination via Partial Message (81 reviews, active, but still draft — needs libp2p/specs merge first)

None actionable yet. `cargo audit`: 1 vulnerability (rsa timing side-channel via jsonwebtoken — no upstream fix, not actionable). Code quality: block verification Gloas paths (BidParentRootMismatch, GloasParentPayloadUnknown, parent payload patching) all have thorough test coverage in gloas.rs (21k lines). Project in maintenance mode.

### run 1913 (Mar 19) — maintenance check, all green

CI green. Nightlies green (latest 2 both green). Clippy clean. Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 10 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843 — none merged. Reviewed #5008 (field name fix in EnvelopesByRoot spec text) — doc-only, our implementation already uses `beacon_block_root` correctly. No new Gloas PRs since last check. Project in maintenance mode.

### run 1912 (Mar 19) — devnet verification after stale state cache fix

Ran full devnet test (`scripts/kurtosis-run.sh`) to verify the stale state cache race fix (a17a399e8). Result: finalized epoch 8 at slot 81, smooth chain progression through Gloas fork at epoch 1. No stalls, no errors. Build clean, clippy clean, CI green. No new spec changes since alpha.3.

### run 1911 (Mar 18) — maintenance check, all green

CI green. Nightlies green (latest 2 both green). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 10 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843 — none merged. Project in maintenance mode.

### run 1909 (Mar 18) — maintenance check, all green

CI green. Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). Checked 2 recent gloas spec path commits: #5001 (parent_block_root in bid filtering key) — already implemented in `observed_execution_bids.rs` (tuple key `(Slot, ExecutionBlockHash, Hash256)` with `is_highest_value_bid`); #5002 (cosmetic wording for self-build payload sig verification) — no code change needed. All 10 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843 — none merged. Project in maintenance mode.

### run 1908 (Mar 18) — maintenance check, all green

CI green. Nightlies green (Mar 18 both green). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits since). No new consensus-specs releases after alpha.3. All 10 tracked Gloas spec PRs still open: #4892 (2 approvals, closest to merge), #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843 — none merged. Verified all 9 fork choice EF test suites pass (including Gloas on_execution_payload and withholding). Project in maintenance mode.

### run 1907 (Mar 18) — maintenance check, all green

CI green (latest push success). Nightlies green (Mar 18 both green). Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 10 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843 — none merged. #4992 (cached PTCs) now labeled `heze` (next fork), still under active discussion. #4747 (Fast Confirmation Rule) updated Mar 17, major feature but not close to merge. Project in maintenance mode.

### run 1906 (Mar 18) — maintenance check, all green

CI green. Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 10 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843 — none merged. Verified our `is_supporting_vote` implementation already matches #4892 (assert message.slot >= block.slot, use == instead of <=). Verified our `get_payload_tiebreaker` already matches #4898 (no PENDING special case at previous slot). Both nightly flakes from Mar 16-17 were already fixed: `finalized_sync_not_enough_custody_peers_on_start` (8f8faa7d removed `expect_empty_network`), `override_backend_with_mdbx_file_present` (rare tmpfs race, existing fsync mitigations thorough). Project in maintenance mode.

### run 1903 (Mar 18) — maintenance check, all green

CI green (latest push success). Nightlies: Mar 18 both green; Mar 17 known flake (`finalized_sync_not_enough_custody_peers_on_start` — timing-sensitive, passes on rerun). Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 10 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843 — none merged. Project in maintenance mode.

### run 1902 (Mar 18) — maintenance check, all green

CI green (latest run success). Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 10 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843 — none merged. No changes since run 1901. Project in maintenance mode.

### run 1901 (Mar 18) — maintenance check, all green

CI green (latest run success: "fix stale state cache race in sync and self-build envelope paths"). Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 10 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747, #4843 — none merged. #4992 (cached PTCs) still contentious — potuz pushing back on caching in state vs client-side caching. Two additional untracked open Gloas PRs: #4630 (EIP-7688 forward-compatible SSZ) and #4840 (EIP-7843 SLOTNUM opcode) — both early stage, not actionable. Project in maintenance mode.

### run 1900 (Mar 18) — maintenance check, all green

CI green (latest run success). Clippy clean (zero warnings). Release build clean (zero warnings). Workspace tests: 4991/4999 pass (8 web3signer failures — external service dependency). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962, #4747 (Fast Confirmation Rule), #4843 (Variable PTC deadline) — none merged. Nightly flakes: Mar 17 `finalized_sync_not_enough_custody_peers_on_start` (fulu network-tests) — timing-sensitive, passes on rerun. Mar 16 `slasher-tests` — filesystem race, passes on rerun. Both subsequent nightlies green. Most recent devnet run (20260318-214930) finalized epoch 8 with stale cache fix. Project in maintenance mode.

### run 1898 (Mar 18) — maintenance check, all green

CI green (latest run success). Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 8 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962 — none merged. No new Gloas PRs opened. No new consensus-specs merges since Mar 15. All remaining TODOs in issue #36 are blocked or non-critical. Project in maintenance mode.

### run 1897 (Mar 18) — maintenance check, all green

CI green (all jobs passing, latest 2 nightly runs green). Build clean (zero compiler warnings). Spec tracked to v1.7.0-alpha.3 (HEAD still 1baa05e from Mar 15 — no new commits). All 8 tracked Gloas spec PRs still open: #4892, #4898, #4992, #4960, #4932, #4939, #5008, #4962 — none merged. No new Gloas PRs opened. All remaining TODOs in issue #36 are blocked or non-critical. Project in maintenance mode.

### run 1896 (Mar 18) — maintenance check, all green

CI green (all jobs passing, latest nightly green). Clippy clean (zero warnings). Build clean (zero compiler warnings). Spec tracked to v1.7.0-alpha.3 (latest release, last spec commit 1baa05e7 from Mar 15 — no new merges). Open Gloas spec PRs: #4892 (2 approvals, still open with discussion), #4898, #4992 (1 approval), #4960, #4932, #4939 — none merged. EF spec tests: 139/139 pass (fake_crypto, minimal_testing). Workspace tests: 4270/4271 pass (1 transient flake in `advertise_false_custody_group_count` — passes in isolation, port conflict under parallel load). Mar 17 nightly fulu network-tests failure was already fixed in 8f8faa7d (stale `expect_empty_network` assertion). web3signer_tests fail as expected (external service dependency). All remaining TODOs in issue #36 are blocked (EIP-7892, blst upstream, PeerDAS) or non-critical. Project in maintenance mode.

### run 1884 (Mar 18) — maintenance check, all green

CI green (all jobs passing). Clippy clean (zero warnings). Build clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (latest release). Reviewed 3 post-alpha.3 merged spec PRs: #5001 (parent_block_root in bid filter — already implemented with 3-tuple key), #5002 (wording fix — no code impact), #5005 (test fixture fix — test-only). Reviewed open spec PRs: #4892 (remove impossible branch — 2 approvals, our impl already matches new logic), #4898 (remove pending status tiebreaker — our impl already handles this correctly), #4992 (cached PTCs in state — major change, still open/discussing). cargo-machete audit: all flagged deps are false positives (TestRandom derive macro needs rand, feature forwarding needs bls). Workspace tests pass (4265/4265, 1 transient failure from dirty Cargo.lock resolved). Project in maintenance mode.

### run 1881 (Mar 18) — maintenance check, all green

CI green (all jobs passing). Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (latest release, HEAD still 1baa05e from Mar 15 — no new merges). Open Gloas spec PRs: #4892 (remove impossible branch), #4898 (remove pending status tiebreaker), #4992 (cached PTCs — 1 approval jtraglia, extensive discussion, not close to merge), #5008 (field name fix — doc-only, our impl already correct), #4962 (sanity/blocks tests — approved by potuz, test-only). No action needed. Project in maintenance mode.

### run 1880 (Mar 18) — maintenance check, all green

CI green. Clippy clean (zero warnings, zero compiler warnings). Spec tracked to v1.7.0-alpha.3 (latest release). Open Gloas spec PRs: #4892 (remove impossible branch), #4898 (remove pending status tiebreaker), #4992 (cached PTCs), #5008 (field name fix) — none merged. New PRs monitored: #4843 (variable PTC deadline — interesting but not merged), #4840 (EIP-7843 SLOTNUM opcode — early stage), #4960/#4932 (new test cases). cargo audit unchanged (rsa medium + 5 unmaintained, non-actionable). cargo-machete audit: all flagged deps are false positives (derive macros, feature forwarding). Dead code audit: all `#[allow(dead_code)]` annotations are legitimate (test infra, error enum variants, lifetime management). Project in maintenance mode.

### run 1871 (Mar 18) — maintenance check, all green

All 218 EF spec tests pass (79 real crypto + 139 fake crypto). CI green (all 26 nightly jobs + 5 CI runs passing). Clippy clean (zero warnings). Spec tracked to v1.7.0-alpha.3 (latest release). Audited 5 post-alpha.3 consensus-specs commits: #5005 (test fix, already handled), #5004 (meta), #4940 (new fork choice tests — our runner already supports `head_payload_status`), #5002 (wording), #5001 (parent_block_root in bid filter — already implemented with 3-tuple). Open Gloas spec PRs: #4892, #4898, #4992, #5008 — none merged. cargo audit unchanged. Project in maintenance mode.

### run 1864 (Mar 18) — full audit pass, no new work

All 218 EF spec tests pass (79 real crypto + 139 fake crypto). CI green. Clippy clean. Spec tracked to v1.7.0-alpha.3 (latest release, 0 unaudited commits). Cross-verified `process_execution_payload_envelope` implementation against spec — all verification steps and state mutations match. Both recent nightly flakes confirmed resolved: range sync test (`expect_empty_network` removed in peer group tracking changes), slasher test (filesystem race on CI runners, existing mitigations thorough). cargo audit unchanged (rsa medium + 5 unmaintained, all non-actionable). Open Gloas spec PRs monitored: #4892, #4898, #4992, #5008 — none merged. Project in maintenance mode.

### run 1862 (Mar 18) — devnet verification after withdrawal dedup

Standard 4-node devnet passed. Run ID: 20260318-125511. Finalized epoch 8 in 468s. Gloas fork at epoch 1, chain healthy through epoch 10. Verified latest commit (deduplicate gloas withdrawal computation into shared function). CI: all green (6/6 jobs passing on latest push). Clippy clean (zero warnings). cargo audit: 1 medium severity in transitive dep `rsa` (no fix available), 5 unmaintained warnings in transitive deps — all non-actionable. No new spec merges since last audit. Open Gloas spec PRs monitored: #4892 (2 approvals), #4898 (1 approval), #4992 (1 approval), #5008 (open).

### run 1846 (Mar 18) — routine devnet verification

Standard 4-node devnet passed. Run ID: 20260318-104308. Finalized epoch 8 in 468s. Gloas fork at epoch 1, chain healthy through epoch 10. CI: all green (push + nightly). Clippy clean. No new spec merges. PR #4992 (cached PTCs) seeing pushback from Grandine on whether it belongs in spec at all.

### run 1827 (Mar 18) — status check (no devnet run)

No code changes to test. Verified: CI green (all jobs passing), clippy clean, no new spec merges. Both recent nightly flakes (range test, slasher test) confirmed fixed in current HEAD. PR #4992 (cached PTCs) studied and documented — ready to implement when merged.

### run 1825 (Mar 18) — routine devnet verification

Standard 4-node devnet passed. Run ID: 20260318-063825. Finalized epoch 8. Gloas fork at epoch 1, chain healthy through epoch 9+. Build clean (zero warnings). Spec tracked — no new merged Gloas PRs since last audit. Open spec PRs (#5008, #4992, #4954, #4843) monitored but not yet merged.

### run 1824 (Mar 18) — routine devnet verification

Standard 4-node devnet passed. Run ID: 20260318-061518. Finalized epoch 8 in 468s. Gloas fork at epoch 1, chain healthy through epoch 10. Docker build cache corruption required `docker builder prune` before successful build. Build clean (zero compiler warnings). Spec tracked — all merged Gloas PRs audited, no new ones since #5002 (Mar 13). Open spec PRs (#5008, #4992, #4954, #4898, #4892) monitored but not yet merged.

### run 1823 (Mar 18) — routine devnet verification

Standard 4-node devnet passed. Run ID: 20260318-054816. Finalized epoch 8 in 468s. Gloas fork at epoch 1, chain healthy through epoch 9. Build clean (zero clippy warnings, zero compiler warnings). Spec tracked to v1.7.0-alpha.3 (latest). No new spec commits since March 15.

### run 1789 (Mar 17) — routine devnet verification

Standard 4-node devnet passed. Run ID: 20260317-230450. Finalized epoch 8 in 468s. Gloas fork at epoch 1, chain healthy through epoch 10. Verified after recent envelope-request-from-attestations changes.

### run 1467 (Mar 16) — routine devnet verification

Standard 4-node devnet passed. Run ID: 20260316-082012. Finalized epoch 8 in 516s. Gloas fork at epoch 1, chain healthy through epoch 11. No issues.

### 2026-02-25 — Genesis sync test (run 108)

**Implemented the syncing devnet test scenario** — the top-priority item from PLAN.md priority 5.

**What was built:**

1. **`kurtosis/vibehouse-sync.yaml`** — Kurtosis config for sync testing:
   - 2 validator nodes (keep running, produce blocks)
   - 2 non-validator nodes (sync targets — stopped and restarted)
   - Same ePBS geth, Gloas fork at epoch 1, minimal preset

2. **`--sync` flag in `kurtosis-run.sh`** — Two-phase sync test:
   - **Phase 1 (finalization):** Start all 4 nodes, immediately stop the 2 non-validators, wait for validators to finalize past epoch 4 (well past the Gloas fork at epoch 1). This creates a chain with both pre-fork (Fulu) and post-fork (Gloas/ePBS) blocks
   - **Phase 2 (sync verification):** Restart EL first (CL needs EL), then CL. Poll both non-validator nodes every 6s (one slot), monitoring:
     - Standard sync API (`/eth/v1/node/syncing` — head_slot, is_syncing)
     - Lighthouse-specific sync state (`/lighthouse/syncing` — SyncingFinalized, SyncingHead, Synced)
   - **Success criteria:** Both nodes report `is_syncing: false` with non-zero head_slot
   - **Post-sync verification:** Queries finality checkpoints and fork version on both sync targets to confirm they're on the Gloas fork with correct finalization

**Key design decisions:**
- Stops both CL and EL for non-validators (not just CL) — ensures the EL also needs to sync, testing the full stack
- Restarts EL before CL — the CL needs the EL to be responsive for engine API calls during sync
- Separate `sync.log` file for the sync phase — easier debugging than mixing with health.log
- 6-minute sync timeout — generous for minimal preset (only ~32 slots to sync at epoch 4)
- `TARGET_FINALIZED_EPOCH=4` (not 8) — enough finalization to prove the chain works, but faster startup for the interesting sync phase

**What this tests:**
- Range sync across the Gloas fork boundary (Fulu blocks → Gloas blocks)
- Block processing pipeline for both pre-fork and post-fork blocks during sync
- State transition through the fork upgrade during catch-up
- ePBS-specific sync behavior (blocks with bids but no inline payload, envelope processing)
- EL sync coordination (engine API during catch-up)

**Usage:**
```bash
scripts/kurtosis-run.sh --sync              # Full test
scripts/kurtosis-run.sh --sync --no-build   # Skip Docker build
scripts/kurtosis-run.sh --sync --no-teardown # Leave running for inspection
```

### 2026-02-26 — Node churn test (run 109)

**Implemented the node churn devnet test scenario** — kill a validator node mid-run, verify the chain continues finalizing, restart it, verify recovery.

**What was built:**

**`--churn` flag in `kurtosis-run.sh`** — Four-phase churn test using the default 4-node config:

- **Phase 1 (warm-up):** Start all 4 validator nodes, wait for finalization to epoch 3 (past Gloas fork at epoch 1). Uses the standard `vibehouse-epbs.yaml` config (no separate config needed).

- **Phase 2 (kill + verify continued finalization):** Stop node 4 (both CL and EL). Wait for finalization to advance at least 2 more epochs with only 3/4 nodes running (75% of stake). This proves the chain handles validator loss gracefully.

- **Phase 3 (restart):** Restart EL first (CL needs EL), then CL — same pattern as sync test.

- **Phase 4 (verify recovery):** Poll the restarted node every 6s until it reports `is_syncing: false` with non-zero `head_slot`. Also monitors the Lighthouse-specific sync state (`/lighthouse/syncing`). After recovery, verifies finality checkpoints on the restarted node.

**Key design decisions:**
- Reuses default `vibehouse-epbs.yaml` config — no separate config file needed since all 4 nodes are identical validators
- `TARGET_FINALIZED_EPOCH=3` for warm-up — enough to prove chain is healthy, faster than the normal target of 8
- `CHURN_FIN_TARGET = PRE_CHURN_FINALIZED + 2` — requires 2 additional finalized epochs with node down, proving sustained chain health
- 3-minute timeout for continued finalization (2 epochs at ~96s each in minimal preset)
- 6-minute timeout for node recovery
- Separate `churn.log` file for all churn phase output
- Kills node 4 specifically (last node) — consistent with sync test pattern

**What this tests:**
- Chain resilience to validator node loss (75% stake threshold for finalization)
- Continued block production and finalization with reduced validator set
- Node recovery after being offline (range sync back to chain head)
- ePBS envelope processing during recovery (blocks with bids during the offline period)
- EL+CL coordination during restart

**Usage:**
```bash
scripts/kurtosis-run.sh --churn              # Full test
scripts/kurtosis-run.sh --churn --no-build   # Skip Docker build
scripts/kurtosis-run.sh --churn --no-teardown # Leave running for inspection
```

### 2026-02-26 — Mainnet preset test (run 110)

**Implemented the mainnet preset devnet test scenario** — run with realistic mainnet parameters instead of the fast minimal preset.

**What was built:**

1. **`kurtosis/vibehouse-mainnet.yaml`** — Kurtosis config for mainnet preset:
   - 4 nodes (vibehouse CL + geth EL), same as default
   - `preset: mainnet` — 32 slots/epoch, 12s/slot, TARGET_COMMITTEE_SIZE=128, PTC_SIZE=512
   - `num_validator_keys_per_node: 128` — 512 total validators
   - Gloas fork at epoch 1 (slot 32)

2. **`--mainnet` flag in `kurtosis-run.sh`** — Overrides timing constants:
   - `SLOTS_PER_EPOCH=32`, recalculates `GLOAS_FORK_SLOT`
   - `POLL_INTERVAL=24` (2 mainnet slots)
   - `TARGET_FINALIZED_EPOCH=4` (past Gloas fork)
   - `TIMEOUT=2400` (40 minutes — mainnet epochs are ~6.4 min each)

**Key design decisions:**
- 512 validators (128/node) — enough for meaningful committee sizes, though smaller than real mainnet. With mainnet TARGET_COMMITTEE_SIZE=128, we get 1 committee per slot (512 / (32 × 128) ≈ 0.125, clamped to 1).
- 40-minute timeout — mainnet finalization is much slower: ~6.4 min/epoch, and we need 4+ epochs to justify + finalize past the Gloas fork.
- No dora (explorer) — reduces resource overhead for the longer-running test.
- Same health polling loop — the script's generic health check (finalization tracking, stall detection) works with any preset.

**What this tests:**
- Mainnet-preset committee sizes and committee assignment logic
- PTC dynamics with PTC_SIZE=512 (vs 2 in minimal)
- Longer epoch times (attestation aggregation over 32 slots)
- Gloas fork transition at realistic timing (slot 32 instead of slot 8)
- General chain health with heavier compute per epoch

**Usage:**
```bash
scripts/kurtosis-run.sh --mainnet              # Full test (~40 min)
scripts/kurtosis-run.sh --mainnet --no-build   # Skip Docker build
scripts/kurtosis-run.sh --mainnet --no-teardown # Leave running for inspection
```

### 2026-02-26 — Long-running test (run 111)

**Implemented the long-running devnet test scenario** — sustained chain health for 50 epochs with periodic resource monitoring.

**What was built:**

**`--long` flag in `kurtosis-run.sh`** — Extended run using the default 4-node config:

- `TARGET_FINALIZED_EPOCH=50` — ~50 epochs × 48s/epoch ≈ 40 min in minimal preset
- `TIMEOUT=3000` (50 min) — generous margin for the long run
- Periodic resource monitoring: every 5th poll (~60s), samples `docker stats` for all CL/EL containers
- Resource snapshots logged to `resources.log` with container name, memory usage, and CPU %
- Memory usage summary printed to stdout alongside chain health

**Key design decisions:**
- Reuses default `vibehouse-epbs.yaml` config — no separate config needed
- 50-epoch target ensures ~40 min of continuous chain operation, well past the "30+ min" goal
- Resource monitoring via `docker stats --no-stream` — non-intrusive, captures memory and CPU per container
- Separate `resources.log` — easy to grep for memory trends over the run duration
- Same stall detection as other modes — if chain stops advancing for 3 consecutive polls, test fails

**What this tests:**
- Memory leak detection over sustained operation (40 min of block production, attestation, finalization)
- Chain stability over many epochs (50 epochs with continuous block production)
- State management under sustained load (state cache behavior over many slots)
- Resource usage trends (growing memory = potential leak)

**Usage:**
```bash
scripts/kurtosis-run.sh --long              # Full test (~40 min)
scripts/kurtosis-run.sh --long --no-build   # Skip Docker build
scripts/kurtosis-run.sh --long --no-teardown # Leave running for inspection
```

### 2026-02-26 — Network partition test (run 112)

**Implemented the network partition devnet test scenario** — simulate a network split by stopping 2/4 nodes, verify finalization stalls, then heal and verify recovery.

**What was built:**

**`--partition` flag in `kurtosis-run.sh`** — Four-phase partition test using the default 4-node config:

- **Phase 1 (warm-up):** Start all 4 validator nodes, wait for finalization to epoch 3 (past Gloas fork).

- **Phase 2 (partition — verify stall):** Stop nodes 3 and 4 (both CL and EL). With only 50% of stake online, finalization should stall because Casper FFG requires 2/3 supermajority. Wait 120s (~2.5 epochs) and verify `finalized_epoch` does NOT advance. The chain should still produce blocks but cannot justify or finalize.

- **Phase 3 (heal):** Restart all stopped nodes (EL first, then CL).

- **Phase 4 (verify recovery):** Wait for finalization to advance at least 2 epochs past the stalled point, proving the chain recovers from the partition and can finalize again once the supermajority is restored.

**Key design decisions:**
- Stops 2 out of 4 nodes — exactly 50% stake, guaranteed below 2/3 threshold for finalization
- 120s stall verification — 2.5 minimal-preset epochs, long enough to confirm finalization doesn't advance
- Separate `partition.log` file for all partition phase output
- WARNING (not failure) if finalization advances during partition — edge case if timing overlaps with justification
- 6-minute timeout for heal phase

**What this tests:**
- Casper FFG correctness — finalization requires 2/3 supermajority, should NOT finalize with 50%
- Chain liveness during partition — blocks should still be produced by remaining 50%
- Fork resolution after partition healing — rejoined nodes reconcile chain state
- Finalization resumption — chain recovers from non-finalizing state when supermajority is restored
- ePBS behavior during degraded operation (bids/envelopes with reduced validator set)

**Usage:**
```bash
scripts/kurtosis-run.sh --partition              # Full test
scripts/kurtosis-run.sh --partition --no-build   # Skip Docker build
scripts/kurtosis-run.sh --partition --no-teardown # Leave running for inspection
```

### 2026-02-26 — Builder path test (run 113)

**Implemented the external builder (ePBS) devnet test scenario** — the last remaining priority-5 devnet test.

**What was built:**

**1. `--genesis-builders N` CLI flag for beacon_node** — injects N builders directly into the genesis Gloas state using deterministic interop keypairs (same scheme as integration tests). Builders start at keypair index `validator_count` and are immediately active (deposit_epoch=0).

- `beacon_node/src/cli.rs` — added `--genesis-builders` CLI arg
- `beacon_node/src/config.rs` — parses `genesis-builders` → `client_config.genesis_builders`
- `beacon_node/client/src/config.rs` — added `genesis_builders: usize` field
- `beacon_node/client/src/builder.rs` — calls `inject_genesis_builders()` during genesis state initialization

**2. `lcli submit-builder-bid` subcommand** — signs and submits a full builder bid with proper proposer preferences:

1. Queries proposer duties for the target epoch to find the actual proposer for the slot
2. Signs `ProposerPreferences` with the proposer's interop keypair
3. Submits preferences to `POST /eth/v1/beacon/pool/proposer_preferences` (new HTTP endpoint)
4. Signs `ExecutionPayloadBid` with the builder's interop keypair
5. Submits bid to `POST /eth/v1/builder/bids`

**3. `POST /eth/v1/beacon/pool/proposer_preferences` HTTP endpoint** — accepts `SignedProposerPreferences`, verifies signature against the validator's pubkey, inserts into the proposer preferences pool. Intended for devnet testing without requiring the P2P gossip path.

- `beacon_node/http_api/src/lib.rs` — added endpoint
- `common/eth2/src/lib.rs` — added `post_beacon_pool_proposer_preferences` client method

**4. `kurtosis/vibehouse-builder.yaml`** — 4-node devnet with `--genesis-builders=1` in `cl_extra_params`.

**5. `--builder` flag in `kurtosis-run.sh`** — Four-phase test:
- **Phase 1:** Wait for finalization to epoch 3 (past Gloas fork at epoch 1)
- **Phase 2:** Submit 3 bids via `lcli submit-builder-bid`, verify at least one is accepted
- **Phase 3:** Wait for 2 more finalized epochs, confirm chain health after bid submission
- **Phase 4:** Check recent blocks for external `builder_index` (non-self-build) in bid field

**Key design decisions:**
- Genesis builder injection bypasses the deposit contract flow — simpler than coordinating Eth1 deposits in the devnet
- The `submit-builder-bid` tool submits proposer preferences first (same fee_recipient + gas_limit) so bid validation passes
- `POST /eth/v1/beacon/pool/proposer_preferences` validates the signature but skips the gossip-specific `proposer_lookahead` check — simpler for the devnet testing use case
- Finding external bids in recent blocks is non-fatal (they may have been for future slots or not won the fork choice) — bid acceptance is the primary verification
- Builder keypairs at indices `[validator_count, validator_count+N)` — consistent with integration test patterns

**What this tests:**
- Builder registration and activation in the genesis Gloas state
- Proposer preferences submission and pool storage
- Bid signature verification against the builder registry pubkey
- Bid validation (slot, execution_payment, fee_recipient match, gas_limit match, parent_block_root, equivocation check)
- Fork choice importing of verified external bids
- P2P gossip propagation of verified bids

**Usage:**
```bash
scripts/kurtosis-run.sh --builder              # Full test
scripts/kurtosis-run.sh --builder --no-build   # Skip Docker build (reuse vibehouse:local)
scripts/kurtosis-run.sh --builder --no-teardown # Leave running for inspection
```

### 2026-02-26 — Payload withholding test (run 114)

**Implemented the payload withholding devnet test scenario** — tests ePBS fork choice EMPTY path.

**What was built:**

**`--withhold` flag in `kurtosis-run.sh`** — Two-phase test using the builder config:

- **Phase 1 (finalize):** Start all 4 nodes (with 1 genesis builder), wait for finalization to epoch 3 (past Gloas fork at epoch 1). Same as builder mode warm-up.

- **Phase 2 (withhold + verify EMPTY path):**
  1. Submit 1 bid via `lcli submit-builder-bid` — beacon node accepts the bid and imports it to fork choice with `payload_revealed=false`
  2. Never submit an envelope — the builder "withholds" the payload
  3. Wait 3 minutes (~2 epochs) for finalization to advance by 2 epochs
  4. **Success criterion**: Chain continues finalizing. This proves fork choice took the EMPTY path (attesters voted `payload_present=false`), not blocking on the missing envelope.
  5. **Failure criterion**: Chain stalls (no finalization for 3 minutes) — would indicate fork choice is incorrectly blocked on the withheld envelope.

**Key design decisions:**
- Reuses `kurtosis/vibehouse-builder.yaml` — same genesis builder config, no new config file needed
- The lcli bid tool submits a bid with `block_hash=zero` (no real EL payload), so a valid envelope is physically impossible — the withholding is guaranteed
- The primary success criterion is **continued finalization**, not "bid rejected" — the chain should work regardless of whether the bid is accepted
- Only 1 bid (vs 3 in builder mode) — we don't need multiple withholding bids, one is sufficient to test the EMPTY path
- `WITHHOLD_FIN_TARGET = PRE_WITHHOLD_FINALIZED + 2` — requires 2 additional finalized epochs, proving sustained liveness on the EMPTY path

**What this tests:**
- Fork choice EMPTY path: when `payload_revealed=false`, attesters can vote `payload_present=false` and the chain moves forward
- Chain liveness under adversarial builder behavior (payload withholding attack)
- PTC attestation mechanism: validators in PTC vote against the withheld payload
- No consensus deadlock: fork choice doesn't block waiting for a builder that never reveals
- The economic disincentive working correctly: builder loses payment when EMPTY path wins

**Usage:**
```bash
scripts/kurtosis-run.sh --withhold              # Full test
scripts/kurtosis-run.sh --withhold --no-build   # Skip Docker build (reuse vibehouse:local)
scripts/kurtosis-run.sh --withhold --no-teardown # Leave running for inspection
```

### 2026-02-26 — Slashing detection test (run 115)

**Implemented the slashing detection devnet test scenario** — the last remaining devnet scenario.

**What was built:**

**1. `lcli inject-slashing` subcommand** — creates and submits proposer or attester slashings to a beacon node using deterministic interop keypairs:

- `lcli/src/inject_slashing.rs` — full implementation:
  - **Proposer slashing**: fetches head block header, creates two conflicting `BeaconBlockHeader`s for the same slot (same proposer, different `state_root`), signs both with `DOMAIN_BEACON_PROPOSER`, submits via `POST /eth/v1/beacon/pool/proposer_slashings`
  - **Attester slashing**: creates two `AttestationData` with same target epoch but different committee index (double-vote), signs both with `DOMAIN_BEACON_ATTESTER`, selects Base vs Electra variant based on current fork name, submits via `POST /eth/v1/beacon/pool/attester_slashings/plain`
  - Fetches fork, genesis_validators_root, and head_slot from beacon API at runtime

- `lcli/src/main.rs` — `inject-slashing` subcommand with `--beacon-url`, `--type`, and `--validator-index` args

**2. `--slashings` flag in `kurtosis-run.sh`** — Four-phase test using the default 4-node config:

- **Phase 1 (warm-up):** Start all 4 validator nodes, wait for finalization to epoch 3 (past Gloas fork at epoch 1). Ensures fork-specific slashing type selection works correctly.

- **Phase 2 (injection):** Inject proposer slashing for validator 1 (double-proposal), then attester slashing for validator 2 (double-vote), with a 1-slot gap between them. Checks acceptance by the beacon pool.

- **Phase 3 (liveness verification):** Wait for chain to continue finalizing by 2 more epochs after slashing injection. Proves the chain doesn't stall when processing slashings.

- **Phase 4 (detection verification):** Query `/eth/v1/beacon/states/head/validators/{idx}` for each slashed validator. Checks `slashed=true` in the validator state. Fails if slashing was accepted by the pool but the validator is not marked slashed.

**Key design decisions:**
- Validators 1 and 2 (not 0) — validator 0 is more likely to be proposer for genesis block, safer to target 1/2
- Gloas fork uses Electra slashing types — `fork_name.electra_enabled()` selects the correct variant at runtime
- Liveness check before detection check — ensures chain health takes priority; detection is secondary
- Failure condition: injected but not detected. If injection fails (e.g., API down), it's a warning not a failure.
- `lcli` found via `$REPO_ROOT/target/release/lcli` first, then PATH fallback — works with devnet from repo root

**What this tests:**
- Proposer slashing pool ingestion and inclusion in blocks
- Attester slashing (double-vote) pool ingestion and inclusion in blocks
- Slashing processing in state transitions (slash_validator, apply_penalties)
- Chain liveness during slashing processing (2 validators losing 1/32 balance each)
- Fork-correct slashing type selection (Electra format in Gloas fork)
- Beacon API slashed status reporting

**Usage:**
```bash
scripts/kurtosis-run.sh --slashings              # Full test
scripts/kurtosis-run.sh --slashings --no-build   # Skip Docker build (reuse vibehouse:local)
scripts/kurtosis-run.sh --slashings --no-teardown # Leave running for inspection
```

### 2026-03-14 — Sync devnet validation (run 1231)

**Validated sync devnet after range sync envelope fixes.**

Ran `--sync` test to verify the 5 commits pushed in this session:
1. Skip pruning Gloas execution payloads (needed to serve envelopes during range sync)
2. Simplify custody range sync peer check (remove epoch requirement)
3. Fix envelope-by-root RPC request size limit (split count vs byte-size fields)
4. Fix sync mode detection (compare head slots within 4-slot threshold)
5. Clippy fix

**Result**: Both supernode and fullnode synced through Gloas fork boundary in 25s. Finalized to epoch 5. Both reported correct Gloas fork version (0x80000038).

### run 1845 (Mar 18) — fix envelope signature race at fork boundary

**Problem**: Devnet stalled at slot 8 (Gloas fork boundary). Envelope gossip verification used `canonical_head.cached_head()` state to compute signing domain. A 2ms race between block import and cached_head update caused verification to use stale Fulu-fork state while VC signed with Gloas-fork domain → all envelopes rejected.

**Root cause**: `execution_payload_envelope_signature_set` computed fork from `state.fork()`, but gossip verifier's state came from cached_head which lagged behind fork choice updates.

**Fix**: Added explicit `fork: &Fork` parameter to `execution_payload_envelope_signature_set`. Gossip/HTTP callers pass `spec.fork_at_epoch(envelope_epoch)` (always correct); state transition callers pass `state.fork()` (correct in their context).

**Tests**: 88 envelope processing, 18 signature set, 422 beacon_chain (Gloas), 204 network — all pass. Clippy clean. Devnet finalized to epoch 8 with clean fork transition.
