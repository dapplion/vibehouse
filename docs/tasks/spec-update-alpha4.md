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
| #5044 | Speed up `compute_ptc` | **Merged 2026-03-26.** Pre-fetches effective_balances before loop, changes `compute_balance_weighted_acceptance` to take balance directly. **Already implemented** — vibehouse pre-computes flat indices + effective_balances arrays before the selection loop (gloas.rs:465-477). No code changes needed. |
| #4840 | Add support for EIP-7843 to Gloas | Open (2026-01-15). Raises blob throughput limits. Not merged — will implement when merged. |
| #4630 | EIP-7688: Forward compatible SSZ types in Gloas | Open (2025-10-01). StableContainer/Profile types for light client compatibility. Not merged — will implement when merged. |
| #4747 | Fast Confirmation Rule | Open, 79 commits, actively debated. Design doc: `docs/workstreams/fast-confirmation-rule.md`. 6 new Store fields, ~25 functions, Gloas compatibility needed. Will implement when merged. |

## Test Vectors

v1.7.0-alpha.4 tag exists (commit a9bc79a7, pushed 2026-03-25T19:22Z) but no GitHub Release published yet (latest release = v1.7.0-alpha.3). Spec-test-check workflow will auto-detect when it's published. Current pinned version: v1.7.0-alpha.3. EF test vectors also not updated (latest: v1.6.0-beta.0 from Sep 2025). Upstream nightly spec test generation has been failing since 2026-03-08 (cancelled runs).

Run 2439: Audited all PRs merged since run 2438 — #5035, #4962, #5023, #4939 all already handled (no code changes needed). Devnet verified: 4-node finalized epoch 8 with all recent proactive implementations (variable PTC deadline, bid gossip relaxation, same-epoch preferences) working correctly. CI green.

Runs 2440-2460: Continuous monitoring, no new spec PRs merged. Key observations: #4843 (variable PTC deadline) received negative analysis from ethDreamer; #4979 was briefly closed for #4992 then reopened; #4992 (alternative approach) was closed. Heze fork assessment (run 2454): FOCIL spec complete but too early for implementation. Devnet verified multiple times (finalized epoch 8). All audits clean (clippy, cargo audit, zero unwrap() in production paths). No actionable work.

Run 2879: New PR #5044 opened (2026-03-26) — "Speed up compute_ptc". Pre-fetches effective_balances before selection loop. Proactively implemented the optimization in vibehouse: flatten committees into parallel indices/effective_balances Vecs, use indexed lookups in the loop instead of repeated state.validators() access. Also added #4840 (EIP-7843 blob limits) and #4630 (EIP-7688 forward-compatible SSZ) to monitoring list. All state_processing tests (1033) and EF spec tests (36/36) pass.

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
| ~~#5044~~ | ~~Speed up `compute_ptc`~~ | **MERGED 2026-03-26.** Pre-fetches effective_balances, changes `compute_balance_weighted_acceptance` signature. **Already implemented** — vibehouse pre-computes effective_balances (gloas.rs:465-477). No code changes needed. |
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

Run 2619: No new spec PRs merged since run 2479 (Mar 26). No newly opened Gloas PRs. All open Gloas PRs unchanged. v1.7.0-alpha.4 release still not published. EF spec-test vectors still v1.6.0-beta.0. CI green. Clippy clean (zero warnings). No actionable work — holding pattern continues.

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

Runs 2490-2609: Continuous monitoring, no new Gloas-relevant spec PRs merged. Summary of notable events during this period:
- Run 2501: Verified #4979 implementation matches final merged spec (a196ff3e) — all 6 components correct. Devnet verified: 4-node finalized epoch 8.
- Run 2510: Full health check — 140/140 EF tests, 9/9 fork choice tests, all proactive implementations confirmed correct.
- Run 2547: Audit gap fix — found #5001 (bid key 3-tuple, already implemented) and #5002 (doc-only) missed in original audit.
- Run 2568: Reviewed #4954 (milliseconds refactor) — large change, not implementing proactively.
- Status throughout: All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). #5036 effectively dead (both author and reviewer opposed). #4747 still conflicting. v1.7.0-alpha.4 tag exists (a9bc79a7) but no GitHub release published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI green, clippy zero warnings, cargo audit unchanged (1 rsa vuln, 5 unmaintained). 10 remaining TODOs all blocked on external deps.

Run 2610 (2026-03-26): No new spec PRs merged since run 2609 (latest merged: #5042 Mar 25, Python dep update). All open Gloas PRs unchanged. v1.7.0-alpha.4 release still not published. CI green, nightly green. No actionable work. Consolidated run log (runs 2490-2609 were 40+ repetitive monitoring entries).

Run 2880 (2026-03-26): No new spec PRs merged since run 2879. #5044 (Speed up compute_ptc) still open — confirmed it blocks the alpha.4 release (test generation timed out at 12h). Verified our implementation already matches: pre-computed effective_balances Vec (line 465), indexed lookups in loop (line 515). No v1.7.0-alpha.4 tag yet (version bump PR merged but release action timed out). All open Gloas PRs unchanged. CI in progress for latest commit (check+clippy passed, 5 jobs running). No actionable work.

Runs 2611-2628 (2026-03-26): No new spec PRs merged. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green, nightly green, clippy zero warnings, cargo audit unchanged (1 rsa vuln no fix, 5 unmaintained). No actionable work.

Run 2629 (2026-03-26): No new spec PRs merged or opened since run 2628. All open Gloas PRs unchanged. **FCR research**: Created comprehensive design doc at `docs/workstreams/fast-confirmation-rule.md` analyzing PR #4747 (8157 additions, 22 files). Key findings: 6 new Store fields, ~25 new functions, CONFIRMATION_BYZANTINE_THRESHOLD=25 config constant, significant integration complexity (per-slot timing hook, head caching for performance, Gloas ForkChoiceNode compatibility). Implementation requires PR merge first — still in active review with 79 commits and ongoing debate.

Runs 2630-2639 (2026-03-26): No new spec PRs merged since run 2629. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0 (spec-test release still v1.5.0). CI green, nightly green (Mar 25). cargo audit unchanged (1 rsa vuln, 5 unmaintained). cargo outdated: only rand_xorshift 0.4→0.5 (test utility, not worth updating). All 10 remaining TODOs blocked on external deps (EIP-7892 ×3, blst, PeerDAS, lmdb). No actionable work.

Run 2640 (2026-03-26): No new spec PRs merged or opened since run 2639. All open Gloas PRs unchanged. v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green, nightly green (Mar 25). Clippy: zero warnings. 9 remaining TODOs all blocked on external deps. No actionable work.

Run 2641 (2026-03-26): No new spec PRs merged since run 2640. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green. No actionable work.

Run 2642 (2026-03-26): No new spec PRs merged since run 2641. All open Gloas PRs unchanged. v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). Workspace tests: 4998/5001 pass, 3 web3signer_tests fail (Consensys artifact server returning HTTP 402 — external infra issue). CI green, clippy zero warnings. No actionable work.

Run 2643 (2026-03-26): No new spec PRs merged since run 2642. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI green. No actionable work.

Run 2644 (2026-03-26): No new spec PRs merged since run 2643. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green. No actionable work.

Runs 2645-2653 (2026-03-26): No new spec PRs merged since run 2644 (latest merged: #5042 Mar 25). No new Gloas PRs opened. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green, nightly green (Mar 25). EF tests: 140/140 pass. #4747 (FCR) still CONFLICTING, REVIEW_REQUIRED. No actionable work.

Runs 2654-2818 (2026-03-26): No new spec PRs merged (latest merged: #5042 Mar 25). No new Gloas PRs opened. All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). #4747 (FCR) active review discussion (mkalinin, etan-status, 0xMushow — optimistic sync interaction, new test vectors fcr-fulu-8241c1d7) but still dirty/conflicting with 146 review comments, not close to merge. v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI green (ci, nightly, spec-test-check all passing Mar 26). Clippy zero warnings. cargo audit unchanged (1 rsa vuln, 5 unmaintained). 10 remaining TODOs all blocked on external deps. Notable: alloy crates updated 1.8.1→1.8.2 (run 2805). No actionable work.

Run 2819 (2026-03-26): New PR opened: **#5044** "Speed up `compute_ptc`" (jtraglia, created 2026-03-26). Optimization to `compute_balance_weighted_acceptance` — changes signature from `(state, index, seed, i)` to `(effective_balance, seed, i)` by pre-computing effective balances list in `compute_balance_weighted_selection`. Primarily a Python performance fix (test generation timed out at 12h for mainnet Gloas tests, blocking alpha.4 release). **No vibehouse code change needed** — our implementation already inlines the acceptance logic and accesses effective_balance via fast Vec indexing. Also added caching wrapper (`cache_this`) for Python test generation — not relevant to us.

Runs 2820-2886 (2026-03-26): Continuous monitoring. Notable events:
- Run 2869: Fixed yanked unicode-segmentation 1.13.1→1.13.2.
- Run 2879: Proactively optimized `compute_ptc` to pre-compute effective balances (matching #5044).
- #5044 still open (0 reviews, blocking alpha.4 release — test generation timeout).
- #5045 opened (remove @always_bls decorator — test infra only).
- #4747 (FCR) got CHANGES_REQUESTED from 0xMushow (optimistic sync interaction).
- All open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630, #5044).
- v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0.
- CI green (ci, nightly, spec-test-check all passing). Zero compiler warnings. cargo audit: 1 rsa vuln, 5 unmaintained (all transitive, not actionable). All remaining TODOs blocked on external deps.

Runs 2887-2903 (2026-03-26): No new spec PRs merged or opened. All open Gloas PRs unchanged (#5044, #5045, #5036, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). #5044 still blocking alpha.4 release (0 reviews). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI fully green (ci, nightly, spec-test-check). Clippy zero warnings. cargo audit: 1 rsa vuln (no fix), 5 unmaintained (all transitive). cargo outdated: only rand_xorshift 0.4→0.5 (incompatible major bump). 8 remaining TODOs all blocked on external deps. No actionable work.

Run 2904 (2026-03-26): No new spec PRs merged or opened since run 2903. #5045 and #5044 updated but still open. All open Gloas PRs unchanged. v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI fully green. No actionable work.

Run 2905 (2026-03-26): No new spec PRs merged or opened since run 2904. All open Gloas PRs unchanged (#5044, #5045, #5036, #4843, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI fully green (ci, nightly, spec-test-check). No actionable work.

Run 2906 (2026-03-26): No new spec PRs merged or opened since run 2905. All open Gloas PRs unchanged (#5044, #5045, #5036, #4843, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI fully green (ci, nightly, spec-test-check). No actionable work.

Run 2908 (2026-03-26): No new spec PRs merged or opened since run 2906. All open Gloas PRs unchanged (#5044, #5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI green (1 run in progress). No actionable work.

Run 2909 (2026-03-26): No new spec PRs merged or opened since run 2908. All open Gloas PRs unchanged (#5044, #5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI green (cmake update in progress). Clippy: zero warnings. All deps up to date. No actionable work.

Run 2910 (2026-03-26): No new spec PRs merged or opened since run 2909. All open Gloas PRs unchanged (#5044, #5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI green (1 run in progress). No actionable work.

Run 2911 (2026-03-26): No new spec PRs merged since run 2910. New PR: #5046 "Increase compute_shuffled_index cache from 96 to 65536 entries" (jtraglia) — Python test infra only (LRU cache size in pysetup/spec_builders/phase0.py for faster mainnet preset test generation). Not relevant to vibehouse. #5044 still open (0 reviews, blocking alpha.4 release). All open Gloas PRs unchanged (#5044, #5045, #5046, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI green. No actionable work.

Run 2912 (2026-03-26): No new spec PRs merged or opened since run 2911. All open Gloas PRs unchanged (#5044, #5045, #5046, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI in progress (cmake 0.1.58 update). No actionable work.

Run 2913 (2026-03-26): No new spec PRs merged or opened since run 2912. All open Gloas PRs unchanged (#5044, #5045, #5046, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI green (1 run in progress). No actionable work.

Run 2914 (2026-03-26): No new spec PRs merged or opened since run 2913. #5044 still open (0 reviews, blocking alpha.4 release). #5046 also open. All open Gloas PRs unchanged. v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI green (cmake 0.1.58 update in progress). 10 TODOs all blocked on external deps. No actionable work.

Run 2915 (2026-03-26): No new spec PRs merged or opened since run 2914. All open Gloas PRs unchanged (#5044, #5045, #5046, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI: 1 run in progress, previous green. No actionable work.

Run 2916 (2026-03-26): No new spec PRs merged or opened since run 2915. All open Gloas PRs unchanged. v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI in progress (cmake 0.1.58 — check/clippy/ef-tests/network passed, beacon_chain/unit/http_api still running). Nightly green (3 consecutive). Clippy zero warnings. cargo audit unchanged. cargo update: 0 packages to update. No actionable work.

Run 2917 (2026-03-26): No new spec PRs merged or opened since run 2916. All open Gloas PRs unchanged (#5044, #5045, #5046, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI in progress (cmake 0.1.58 update). No actionable work.

Run 2918 (2026-03-26): No new spec PRs merged or opened since run 2917. All open Gloas PRs unchanged (#5044, #5045, #5046, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI in progress (cmake 0.1.58 update, 28min). No actionable work.

Run 2919 (2026-03-26): No new spec PRs merged or opened since run 2918. All open Gloas PRs unchanged (#5044, #5045, #5046, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI in progress (cmake 0.1.58 update). No actionable work.

Run 2920 (2026-03-26): No new spec PRs merged or opened since run 2919. All open Gloas PRs unchanged (#5044, #5045, #5046, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI fully green (cmake 0.1.58 update passed). No actionable work.

Run 2921 (2026-03-26): No new spec PRs merged or opened since run 2920. #5046 (compute_shuffled_index cache — Python test infra only) still open. All open Gloas PRs unchanged (#5044, #5045, #5046, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI fully green. No actionable work.

Run 2922 (2026-03-26): **#5044 merged** (2026-03-26T19:03:01Z) — "Speed up compute_ptc". Pre-fetches effective_balances in `compute_balance_weighted_selection`, changes `compute_balance_weighted_acceptance` to take `effective_balance` directly instead of `(state, index)`. **No vibehouse code changes needed** — our implementation already pre-computes effective_balances Vec (gloas.rs:465-477) and uses indexed lookups (gloas.rs:515). #5044 was blocking alpha.4 release (test generation timed out at 12h); with this merged, release should unblock soon. #5045 (remove @always_bls) and #5046 (shuffled_index cache) still open — both test infra only. All other open Gloas PRs unchanged (#4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). CI fully green, nightly green.

Run 2923 (2026-03-26): **#5046 merged** (2026-03-26T19:02:36Z) — "Increase compute_shuffled_index cache from 96 to 65536 entries". Python test infra only (LRU cache size for faster mainnet test generation). No vibehouse code changes needed. No other new spec PRs merged or opened. All open Gloas PRs unchanged (#5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI fully green. No actionable work.

Run 2924 (2026-03-26): No new spec PRs merged or opened since run 2923. All open Gloas PRs unchanged (#5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI fully green. With #5044 and #5046 merged (both unblocking test generation), alpha.4 release should be imminent. No actionable work.

Run 2925 (2026-03-26): No new spec PRs merged or opened since run 2924. All open Gloas PRs unchanged (#5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI fully green. No actionable work.

Run 2926 (2026-03-26): No new spec PRs merged or opened since run 2925. All open Gloas PRs unchanged (#5045, #5036, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI fully green. No actionable work.

Run 2927 (2026-03-26): No new spec PRs merged or opened since run 2926. All open Gloas PRs unchanged (#5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI fully green. No actionable work.

Run 2928 (2026-03-26): No new spec PRs merged or opened since run 2927. All open Gloas PRs unchanged (#5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI fully green. All deps at latest compatible versions (9 behind latest are major version bumps). No actionable work.

Run 2929 (2026-03-26): No new spec PRs merged or opened since run 2928. All open Gloas PRs unchanged (#5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI fully green. No actionable work.

Run 2930 (2026-03-26): No new spec PRs merged or opened since run 2929. All open Gloas PRs unchanged (#5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI fully green. No actionable work.

Run 2931 (2026-03-26): Verified #5035 ("Allow same epoch proposer preferences", merged Mar 25) already fully implemented — gossip validation (gossip_methods.rs:4126) accepts current+next epoch, checks proposal_slot > current_slot; VC broadcasting (duties_service.rs:1676-1706) fetches current-epoch duties and filters future slots. No new spec PRs merged or opened since run 2930. All open Gloas PRs unchanged (#5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0. CI fully green. No actionable work.

Run 2932 (2026-03-26): No new spec PRs merged or opened since run 2931. All open Gloas PRs unchanged (#5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 tag still not published (latest = v1.7.0-alpha.3, latest release = v1.6.1). EF test vectors still v1.6.0-beta.0. CI fully green. No dependency updates available. No actionable work.

Run 2933 (2026-03-26): No new spec PRs merged or opened since run 2932. All open Gloas PRs unchanged (#5045, #4843, #4898, #4892, #4960, #4932, #4954, #4747, #4840, #4630). v1.7.0-alpha.4 release still not published (latest = v1.6.1). EF test vectors still v1.6.0-beta.0. CI fully green. No actionable work.
