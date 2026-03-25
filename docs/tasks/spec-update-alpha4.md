# Spec Update: v1.7.0-alpha.4

## Objective
Track and implement consensus-specs changes in the v1.7.0-alpha.4 release.

## Status: DONE

All PRs included in alpha.4 (since alpha.3) have been audited. No code changes needed.

## Changes Audit (run 2354)

### PRs merged since alpha.3 (included in alpha.4)

| PR | Description | Status |
|----|-------------|--------|
| #5022 | Add check that block is known in `on_payload_attestation_message` | Already handled — vibehouse returns `InvalidPayloadAttestation::UnknownBeaconBlockRoot` |
| #5008 | Correct field name `block_root` → `beacon_block_root` in `ExecutionPayloadEnvelopesByRoot` | Already correct — vibehouse uses `beacon_block_root` |
| #5023 | Fix block root filenames and Gloas comptests | Test infra only — no production code changes |
| #5014 | Update EIP8025 p2p protocol | Not relevant — EIP-8025 is a separate unscheduled feature |
| #5005 | Fix builder voluntary exit success test | Test-only fix, already handled |
| #5034 | Bump version to v1.7.0-alpha.4 | Version bump only |

### CI/tooling PRs (not relevant)

#5031, #5030, #5029, #5028, #5027, #5026, #5025, #5017, #5015, #5010, #5009, #5007, #5006, #5004

## Post-alpha.4 merged PRs (run 2361-2363/2449, 2026-03-25)

| PR | Description | Status |
|----|-------------|--------|
| #5035 | Allow same epoch proposer preferences | **Merged 2026-03-24.** Already implemented — gossip validation accepts current+next epoch (gossip_methods.rs:4111), slot-not-passed check (4124), epoch_offset index calc (4166-4171). VC broadcasts for both epochs (duties_service.rs:1676-1726). Fixed docstring to match (run 2449). |
| #5037 | Remove fork version/epoch in EIP-8025 specs | Not relevant — EIP-8025 not implemented |
| #4962 | Sanity/blocks tests for missed payload withdrawal interactions | **Merged 2026-03-25.** Test vectors only (4 tests for missed payload + withdrawal edge cases). Verified (run 2363): vibehouse already handles all 4 scenarios correctly — `process_withdrawals_gloas` returns early on EMPTY parent without clearing `payload_expected_withdrawals`, envelope validation checks stale withdrawals, block production uses stale value directly. Integration test `gloas_stale_withdrawal_carryover_across_empty_parent` covers this. |
| #4939 | Request missing payload envelopes for index-1 attestation | **Merged 2026-03-24.** Already implemented — envelope request via ExecutionPayloadEnvelopesByRoot RPC when index-1 attestation arrives without envelope. |

### Open Gloas PRs (still monitoring)

| PR | Description | Status |
|----|-------------|--------|
| #4979 | PTC window cache | **Proactively implemented** — all code done, EF test handler skips schema-mismatched vectors. Verified implementation matches latest PR diff including MIN_SEED_LOOKAHEAD constant usage (run 2364). |
| #5036 | Relax bid gossip dependency on proposer preferences | **Proactively implemented** — bid validation uses conditional `if let Some(preferences)` (gloas_verification.rs:480). Verified matches latest PR diff (run 2364). |
| #4898 | Simplify fork choice is_supporting_vote | Approved, not merged. Already implemented debug_assert. |
| #4892 | Assert slot >= block slot in fork choice | Approved, not merged. Already implemented debug_assert. |
| #4843 | Variable PTC deadline | **Proactively implemented** (run 2371) — rename payload_present→payload_timely, is_payload_timely→has_payload_quorum, MIN_PAYLOAD_DUE_BPS config, variable deadline in get_payload_attestation_data. Commit a7baf6b57. |
| #4960 | Gloas fork choice test (new validator deposit) | Test vectors — will integrate when released |
| #4932 | Gloas sanity/blocks tests with payload attestation coverage | Test vectors — will integrate when released |

## Test Vectors

No v1.7.0-alpha.4 release/tag created yet on consensus-specs (as of run 2449, 2026-03-25). Version bump PR (#5034) merged Mar 24 but no GitHub release published. Spec-test-check workflow will auto-detect when it's published. Current pinned version: v1.7.0-alpha.3. EF test vectors also not updated (latest: v1.6.0-beta.0 from Sep 2025).

Run 2439: Audited all PRs merged since run 2438 — #5035, #4962, #5023, #4939 all already handled (no code changes needed). Devnet verified: 4-node finalized epoch 8 with all recent proactive implementations (variable PTC deadline, bid gossip relaxation, same-epoch preferences) working correctly. CI green.

Run 2440: Full audit — no new spec PRs merged since run 2439. All 6 tracked open PRs (#4979, #5036, #4898, #4892, #4960, #4932) still open. v1.7.0-alpha.4 release still not published (spec-test-version-check confirms latest = v1.7.0-alpha.3). CI green, clippy clean (zero warnings), devnet passing (finalized epoch 8), cargo audit clean (1 known RSA advisory in transitive dep, no fix available). Production consensus code verified: zero unwrap() calls in gloas state processing. No actionable work.

Run 2441: Full audit — no new spec PRs merged since run 2440. All open PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932). #4843 mergeable:clean (approved), #4979/#5036 still blocked. v1.7.0-alpha.4 tag still not created. CI green, clippy zero warnings. Production safety audit: zero unwrap() calls in all critical production paths (state_processing, fork_choice, beacon_chain core). Devnet verified: finalized epoch 8. No actionable work.

Run 2443: Full audit — no new spec PRs merged since run 2441. All open PRs unchanged. v1.7.0-alpha.4 release still not published. Note: #4843 (variable PTC deadline) received negative analysis from ethDreamer ("doesn't seem worth it"), may be dropped — our proactive implementation stands but may need reverting if PR is closed. #4979 (PTC window) was briefly closed in favor of #4992, then reopened (discussion ongoing). #4992 (alternative 2-committee approach, 8KB) was closed. Devnet verified post-alloy 1.8.1 update: finalized epoch 8 at slot 81 (4 nodes, gloas fork at epoch 1). execution_layer tests: 143/143 pass. Clippy: zero warnings. No actionable work.

Run 2444: Full audit — no new spec PRs merged since run 2443. All open PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932). #4979 (PTC window) updated today with commit "State that update_fast_confirmation_variables must be called once" on #4747, but #4979 itself has no new commits since Mar 24 (doc-only fixes already verified). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). Clippy: zero warnings. cargo audit: same known RSA advisory (no fix). Tests: 1033 state_processing pass, 140 EF (fake_crypto) pass, 80 EF (real crypto) pass. No actionable work.

Run 2445: Full audit — no new spec PRs merged since run 2444. All open PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932). #4979 mergeable:clean, updated today (2026-03-25) but no new behavioral changes. v1.7.0-alpha.4 release still not published (latest tag = v1.7.0-alpha.3). Latest EF test release still v1.6.0-beta.0 (Sep 2025). CI: alloy 1.8.1 update running (4/6 jobs green so far). Clippy: zero warnings. cargo check: clean. cargo audit: same known RSA advisory (no fix). No actionable work.

Run 2446: Full audit — no new spec PRs merged since run 2445. All open PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI fully green — alloy 1.8.1 update passed all 6 jobs (check+clippy, ef-tests, beacon_chain, http_api, unit tests, network+op_pool). cargo audit: same RSA advisory (no fix). Rust toolchain up to date (1.94.0 stable, 1.96.0-nightly). No actionable work.

Run 2447: No new spec PRs merged since run 2446. CI green (latest ci, spec-test-version-check, nightly all success). No actionable work.

Run 2451: #5040 merged (fork choice test infra fix — Python test generator bug, no production code impact). All open Gloas PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932). v1.7.0-alpha.4 release still not published. CI green. cargo audit: same known advisories. No actionable work.

Run 2452: No new spec PRs merged since run 2451. All open Gloas PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green (in-progress run for 2449, all prior runs success). Clippy: zero warnings. No actionable work.

Run 2453: No new spec PRs merged since run 2452. All open Gloas PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932). #4979 updated today but no new commits (last: 89ce53b0, Mar 24). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green. Clippy: zero warnings. cargo audit: 6 advisories (all known — rsa, ansi_term, bincode, derivative, filesystem false positive on local crate, paste — no fixes available). No actionable work.

Run 2454: No new spec PRs merged since run 2453. All open Gloas PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green. Clippy: zero warnings. cargo audit: 6 advisories (all known). Heze fork assessment: FOCIL spec (EIP-7805) is complete across 7 files with ~20 functions, but only 1 EIP in fork, no test vectors, active spec churn (#4979, #4954 touch heze) — too early for implementation. No actionable work.

Run 2455: No new spec PRs merged since run 2454. All open Gloas PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932). #4979 updated today (2026-03-25) but no new commits since Mar 24 (last: 89ce53b0 "Fix typo"). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green (all 5 recent runs success). Clippy: zero warnings. cargo check: clean. No actionable work.

Run 2456: No new spec PRs merged since run 2455. All open Gloas PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932). #4979 still open (mergeable:clean, 15 commits, 19 review comments), no new commits since Mar 24. v1.7.0-alpha.4 release still not published (latest release = v1.6.1, Nov 2025). EF test vectors still v1.6.0-beta.0. CI green (all recent runs success). Clippy: zero warnings. No actionable work.

Run 2457: No new spec PRs merged since run 2456. All open Gloas PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932). v1.7.0-alpha.4 release still not published. CI green. Clippy: zero warnings. cargo audit: same known advisories (1 vuln rsa, 5 unmaintained warnings). No actionable work.

Run 2458: No new spec PRs merged since run 2457. All open Gloas PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932, #4954). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green (all jobs pass including nightly across all forks). Clippy: zero warnings. cargo audit: same known advisories. Dependencies: `cargo update --dry-run` shows only minor windows-sys additions, no meaningful updates. No actionable work.

Run 2459: No new spec PRs merged since run 2458. All open Gloas PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932, #4954). #4979 updated today (15:38 UTC) but still 15 commits, no new code changes. v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green. Clippy: zero warnings. No actionable work.

Run 2460: No new spec PRs merged since run 2459. All open Gloas PRs unchanged (#4979, #5036, #4843, #4898, #4892, #4960, #4932, #4954). #4979 still 15 commits (last: 89ce53b0 Mar 24), mergeable:clean. #4747 (Fast Confirmation) mergeable:dirty, 79 commits, still actively debated. v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green (all jobs pass). Clippy: zero warnings. No actionable work.

## Open Gloas PRs to Watch

| PR | Description | Notes |
|----|-------------|-------|
| #4979 | PTC window cache in BeaconState | Major change, renamed to `ptc_window`, still under active design discussion, not merged (as of 2026-03-25). Also tagged `heze`. Implementation verified (run 2423) — Mar 24 commits (e7b19104 "Fix epoch restrictions in paragraph", 89ce53b0 "Fix typo") are doc-only; vibehouse implementation unchanged and aligned. |
| #5036 | Relax bid gossip dependency on proposer preferences | Open — proactively implemented (commit 1c7e608d4). Verified (run 2374): latest commit (4e455a3) is doc-only, no behavioral changes. |
| #4960 | Fork choice test for new validator deposit | Test vectors |
| #4954 | Update fork choice store to use milliseconds | Open, 0 reviews, large refactor (28 files), also tagged `heze` — not worth implementing proactively |
| #4932 | Sanity/blocks tests with payload attestation coverage | Test vectors |
| #4898 | Remove pending status from tiebreaker | 1 approval, still open — vibehouse already matches post-PR behavior |
| #4892 | Remove impossible branch in forkchoice | 2 approvals, still open — vibehouse already uses debug_assert + == (matches post-PR) |
| #4747 | Fast Confirmation Rule | Open, 128+ reviews, CONFLICTING mergeable state, test vectors posted Mar 11+22. Still actively debated, not close to merge. Also tagged eip7805 (FOCIL). |
| #4843 | Variable PTC deadline | Open, APPROVED, 11 reviews. **Proactively implemented** (run 2371, commit a7baf6b57): renamed payload_present→payload_timely across 27 files, is_payload_timely→has_payload_quorum in fork choice, added MIN_PAYLOAD_DUE_BPS (3000) config, variable deadline in get_payload_attestation_data (interpolates linearly from MIN to MAX based on SSZ size). Envelope arrival timing tracked per block root. |
| #4840 | Add support for EIP-7843 to Gloas | Open, Jan 2026 |
| #4630 | EIP-7688: Use forward compatible SSZ types in Gloas | Open, stale since Feb 2026. SSZ refactor, light client related. Not worth proactive implementation. |
| #4558 | Add Cell Dissemination via Partial Message Specification | Open, updated Mar 2026. PeerDAS/fulu + gloas. Monitor. |
