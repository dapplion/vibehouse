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
| #5004 | Add dependencies section to release notes | Release tooling only |
| #5002 | Make wordings clearer for self build payload signature verification | Documentation-only change in p2p-interface.md — no behavioral change |
| #5001 | Add `parent_block_root` to bid filtering key | Already implemented — vibehouse uses `(Slot, ExecutionBlockHash, Hash256)` 3-tuple key in `observed_execution_bids.rs` |
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
| #4979 | PTC window cache in BeaconState | **Merged 2026-03-25.** Proactively implemented — verified (run 2519) implementation matches final merged spec exactly: `compute_ptc`, `get_ptc_committee` (spec `get_ptc`), `process_ptc_window`, `initialize_ptc_window` all match. EF test handler skips schema-mismatched vectors until new test vectors are released with `ptc_window` field. |
| #5040 | Fix fork choice compliance test `is_early_message` bug | **Merged 2026-03-25.** Test infrastructure only — reversed comparison `<` to `>` in Python test generator. No vibehouse code changes needed. |
| #4558 | Cell dissemination via partial message specification | **Merged 2026-03-25.** Adds `PartialDataColumnHeader` container and validation rules for gossipsub partial messages on `data_column_sidecar_{subnet_id}`. Gloas changes mirror Fulu changes (remove `signed_block_header`, add `slot`+`beacon_block_root`). **Not implemented** — requires upstream rust-libp2p gossipsub partial messages support. Vibehouse does not implement Fulu partial messages either. Will implement when libp2p support is available. |

### Open Gloas PRs (still monitoring)

| PR | Description | Status |
|----|-------------|--------|
| #5036 | Relax bid gossip dependency on proposer preferences | **Reverted (run 2488)** — PR is effectively dead (both author and reviewer oppose it). Restored spec-compliant behavior: bids are IGNORED when proposer preferences haven't been seen. |
| #4898 | Simplify fork choice is_supporting_vote | Approved, not merged. Already implemented debug_assert. |
| #4892 | Assert slot >= block slot in fork choice | Approved, not merged. Already implemented debug_assert. |
| #4843 | Variable PTC deadline | **Proactively implemented** (run 2371) — rename payload_present→payload_timely, is_payload_timely→has_payload_quorum, MIN_PAYLOAD_DUE_BPS config, variable deadline in get_payload_attestation_data. Commit a7baf6b57. |
| #4960 | Gloas fork choice test (new validator deposit) | Test vectors — will integrate when released |
| #4932 | Gloas sanity/blocks tests with payload attestation coverage | Test vectors — will integrate when released |
| #4954 | Update fork choice store to use milliseconds | Open. Converts `Store.time`→`Store.time_ms`, `Store.genesis_time`→`Store.genesis_time_ms`. Not merged yet — will implement when merged. |
| #4747 | Fast Confirmation Rule | Open. Adds `confirmed_root` to fork choice Store. Still in review — not yet actionable. |

## Test Vectors

No v1.7.0-alpha.4 release/tag created yet on consensus-specs (as of run 2519, 2026-03-26). Version bump PR (#5034) merged Mar 24 but no GitHub release published. Spec-test-check workflow will auto-detect when it's published. Current pinned version: v1.7.0-alpha.3. EF test vectors also not updated (latest: v1.6.0-beta.0 from Sep 2025).

Run 2439: Audited all PRs merged since run 2438 — #5035, #4962, #5023, #4939 all already handled (no code changes needed). Devnet verified: 4-node finalized epoch 8 with all recent proactive implementations (variable PTC deadline, bid gossip relaxation, same-epoch preferences) working correctly. CI green.

Runs 2440-2460: Continuous monitoring, no new spec PRs merged. Key observations: #4843 (variable PTC deadline) received negative analysis from ethDreamer; #4979 was briefly closed for #4992 then reopened; #4992 (alternative approach) was closed. Heze fork assessment (run 2454): FOCIL spec complete but too early for implementation. Devnet verified multiple times (finalized epoch 8). All audits clean (clippy, cargo audit, zero unwrap() in production paths). No actionable work.

Run 2461/2463: Fixed HTTP API gap — `post_beacon_pool_proposer_preferences` was missing epoch/slot validation that gossip path already had. Added checks + fixed 8 bid tests using `current_slot + 1`. CI green.

Runs 2464-2468: Monitoring, no new merges. CI green, all open PRs unchanged.

Run 2469-2470: **#4979 merged** (2026-03-25T18:24:01Z). Verified proactive implementation matches final merge commit (a196ff3e) across all 7 behavioral aspects. No code changes needed. EF test handler ready for updated vectors. Devnet verified: finalized epoch 8.

Runs 2471-2475: Monitoring, no new Gloas merges. #4558 (Cell Dissemination) updated but PeerDAS scope.

Run 2476: **#5035 merged** ("Allow same epoch proposer preferences"). No code changes needed — vibehouse already allows current+next epoch in gossip, HTTP API, and VC broadcast.

## Open Gloas PRs to Watch

| PR | Description | Notes |
|----|-------------|-------|
| ~~#4979~~ | ~~PTC window cache in BeaconState~~ | **MERGED 2026-03-25.** Proactively implemented and verified against final merge commit (a196ff3e). EF test handler ready, awaiting updated test vectors. |
| ~~#5035~~ | ~~Allow same epoch proposer preferences~~ | **MERGED 2026-03-25.** Already implemented — no code changes needed. |
| ~~#4558~~ | ~~Cell Dissemination via Partial Messages~~ | **MERGED 2026-03-25.** Adds `PartialDataColumnHeader` + validation for `data_column_sidecar_{subnet_id}`. **Not actionable** — requires Gossipsub partial messages extension (no Rust libp2p implementation). |
| ~~#5036~~ | ~~Relax bid gossip dependency on proposer preferences~~ | **Effectively dead** — both author and reviewer opposed. Proactive implementation reverted (run 2488). |
| #4843 | Variable PTC deadline | Open, APPROVED. **Proactively implemented** (commit a7baf6b57). |
| #4898 | Remove pending status from tiebreaker | Open — vibehouse already matches post-PR behavior. |
| #4892 | Remove impossible branch in forkchoice | Open — vibehouse already uses debug_assert + ==. |
| #4960 | Fork choice test for new validator deposit | Test vectors — will integrate when released. |
| #4932 | Sanity/blocks tests with payload attestation coverage | Test vectors — will integrate when released. |
| #4954 | Update fork choice store to use milliseconds | Open, 0 reviews, large refactor, also tagged `heze` — not implementing proactively. |
| #4747 | Fast Confirmation Rule | Open, 128+ reviews, CONFLICTING. Actively debated, not close to merge. |
| #4840 | Add support for EIP-7843 to Gloas | Open, stale since Jan 2026. |
| #4630 | EIP-7688: Forward compatible SSZ types | Open, stale since Feb 2026. Not implementing proactively. |

Run 2477-2478: #4558 merged, #5041/#5042 merged (Python tooling only). No actionable Gloas changes. v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.5.0. CI green, clippy clean.

Run 2479: No new spec PRs merged since run 2478. All open Gloas PRs unchanged (#5036, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest release = v1.6.1). EF test vectors still v1.5.0. CI fully green. Clippy: zero warnings. cargo audit: same known advisories (1 rsa vuln, 5 unmaintained). 11 TODOs remain in Rust code (all blocked on external deps: EIP-7892 ×3, blst safe API, lmdb, PeerDAS checkpoint sync, misc non-critical). No actionable work.

Run 2480: #5040 merged (fork choice compliance test bug fix — Python test infra only, no production code). All open Gloas PRs unchanged (#5036, #4843, #4898, #4892, #4960, #4932, #4954, #4747). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). CI green. 10 TODOs in Rust (all blocked on external deps). No actionable work.

Run 2481: No new spec PRs merged. All open Gloas PRs unchanged. Notable: #5036 got pushback from jtraglia (DoS concerns), #4843 still debated (ethDreamer skeptical of value). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.5.0. CI green, clippy clean, cargo audit unchanged. No actionable work.

Run 2482: No new spec PRs merged since run 2481. All open Gloas PRs unchanged (#5036, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). #5036 still contested (jtraglia disagrees, waiting for client dev input). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.5.0. CI green, clippy clean, cargo audit unchanged. No actionable work.

Run 2483: No new spec PRs merged since run 2482. Only Python dep updates (#5042 eth-hash, #5041 eth-utils). All open Gloas PRs unchanged (#5036, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). #5036 still contested (jtraglia: "disagree but leave open for client devs"). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.5.0. CI green. No actionable work.

Run 2484: No new spec PRs merged since run 2483. All open Gloas PRs unchanged (#5036, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). #5036 still contested (jtraglia comment unchanged). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.5.0. CI green. No actionable work.

Run 2485: No new spec PRs merged since run 2484. All open Gloas PRs unchanged. v1.7.0-alpha.4 release still not published. CI green. cargo audit unchanged (1 rsa vuln no fix, 5 unmaintained). No actionable work.

Run 2486: No new spec PRs merged since run 2485. All open Gloas PRs unchanged. #5036 update: both author (nflaig) and reviewer (jtraglia) now against the PR — likely to be closed. v1.7.0-alpha.4 release still not published (latest = v1.6.1). CI green. No actionable work.

Run 2487: No new spec PRs merged since run 2486. All open Gloas PRs unchanged (#5036, #4843, #4898, #4892, #4960, #4932, #4954, #4747). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). CI green (nightly Mar 22 failure was transient nextest 404, Mar 23 was slasher dead_code already fixed in 5d23ecf85). Clippy: zero warnings. cargo audit: unchanged (1 rsa vuln no fix, 5 unmaintained). No actionable work.

Run 2527: No new spec PRs merged since run 2487. All open Gloas PRs unchanged (#5036, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). #4747 (Fast Confirmation Rule) updated 2026-03-25 but still actively debated and not close to merge. v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.5.0. CI green. No actionable work.

Run 2488: No new spec PRs merged since run 2487. All open Gloas PRs unchanged (#5036, #4843, #4898, #4892, #4960, #4932, #4954, #4747). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). CI in progress for #5036 revert commit (adf42bf93). Nightly green (Mar 25). No actionable work.

Run 2489: No new spec PRs merged since run 2488. All open Gloas PRs unchanged. Fixed CI failure: stale test `bid_submission_without_proposer_preferences_passes_prefs_check` was left behind from #5036 revert — test expected bids to pass without preferences but revert restored rejection. Removed test, all 15 bid tests pass. v1.7.0-alpha.4 release still not published (latest = v1.6.1). #5036 status: both author and reviewer against it (jtraglia: "disagree but leave open for client devs", nflaig: "also not in favor").

Runs 2490-2500: Continuous monitoring, no new Gloas-relevant spec PRs merged. All 8 open Gloas PRs unchanged (#5036, #4843, #4898, #4892, #4960, #4932, #4954, #4747). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green (all 6 jobs passed for 11b70dfc1). #5036: both author (nflaig) and reviewer (jtraglia) against it. Issue #36: all remaining items blocked on external deps (EIP-7892 ×3, blst safe API, PeerDAS).

Run 2501: Verified #4979 implementation matches final merged spec (a196ff3e, merged 2026-03-25T18:24:01Z) — all 6 components verified (ptc_window field, compute_ptc, get_ptc, initialize_ptc_window, process_ptc_window, epoch ordering). No new Gloas PRs merged since run 2500. Build clean (zero clippy warnings). Devnet verified: 4-node finalized epoch 8, healthy chain through Gloas fork. Consolidated run log entries 2490-2500 (all identical monitoring). No actionable work.

Runs 2502-2535: Continuous monitoring. No new Gloas-relevant spec PRs merged. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). #5036 effectively dead (both author and reviewer opposed). #4747 (Fast Confirmation Rule) updated Mar 25 but still dirty/conflicting with 138 review comments — not close to merge. v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green, clippy zero warnings, cargo audit unchanged (1 rsa no fix, 5 unmaintained). Full health verified in run 2510: 140/140 EF tests, 9/9 fork choice tests, all proactive implementations (#4979, #5035, #4962) confirmed correct. 10 remaining TODOs all blocked on external deps. No actionable work.

Runs 2536-2545: No new spec PRs merged since run 2535. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). #4747 still conflicting (138 review comments, mergeable=false). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green (nightly Mar 25 success), build clean. No actionable work.

Run 2546: No new spec PRs merged since run 2545. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). Latest consensus-specs release still v1.6.1 (no v1.7.0-alpha.4 published). EF test vectors still v1.6.0-beta.0. CI green (all 6 jobs success), nightly green (25/25 jobs success Mar 25). No actionable work.

Run 2547: Audit gap fix — found two Gloas-labeled PRs (#5001, #5002) merged between alpha.3 (Mar 11) and alpha.4 (Mar 24) that were missed in the original audit. **#5001** "Add `parent_block_root` to bid filtering key" (merged Mar 12): changes bid highest-value tracking from `(slot, parent_block_hash)` to `(slot, parent_block_hash, parent_block_root)`. **Already implemented** — vibehouse uses `HashMap<(Slot, ExecutionBlockHash, Hash256), u64>` in `observed_execution_bids.rs:48` with the full 3-tuple key since initial implementation. **#5002** "Make wordings clearer for self build payload signature verification" (merged Mar 13): documentation-only change in p2p-interface.md, references `verify_execution_payload_envelope_signature` instead of describing the check inline. No behavioral change, no code impact. No new Gloas PRs merged since run 2546. All open Gloas PRs unchanged (#4843, #4898, #4892, #4954, #4747, #4840, #4630, #5036). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green. No actionable work.

Run 2548: No new spec PRs merged since run 2547. All open Gloas PRs unchanged (#4843, #4898, #4892, #4954, #4747, #4840, #4630, #5036). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green (latest run success Mar 25). No actionable work.

Runs 2549-2556: No new spec PRs merged since run 2548. All open Gloas PRs unchanged (#4843, #4898, #4892, #4954, #4747, #4840, #4630). #4747 still conflicting (138 review comments, mergeable=false). #5036 still open but effectively dead. v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green (all runs success, nightly Mar 25 green). No actionable work.

Runs 2557-2558: No new spec PRs merged since run 2556. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). #5036 still open but effectively dead. v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green. No actionable work.

Runs 2559-2560: No new spec PRs merged since run 2558. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). #5036 still open but effectively dead (mergeable=blocked). #4747 updated Mar 25 but still dirty/conflicting. v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI green. No actionable work.
