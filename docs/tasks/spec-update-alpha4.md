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

## Post-alpha.4 merged PRs (run 2361-2363/2449/2964, 2026-03-25/26)

| PR | Description | Status |
|----|-------------|--------|
| #5035 | Allow same epoch proposer preferences | **Merged 2026-03-24.** Already implemented — gossip validation accepts current+next epoch (gossip_methods.rs:4111), slot-not-passed check (4124), epoch_offset index calc (4166-4171). VC broadcasts for both epochs (duties_service.rs:1676-1726). Fixed docstring to match (run 2449). |
| #5037 | Remove fork version/epoch in EIP-8025 specs | Not relevant — EIP-8025 not implemented |
| #4962 | Sanity/blocks tests for missed payload withdrawal interactions | **Merged 2026-03-25.** Test vectors only (4 tests for missed payload + withdrawal edge cases). Verified (run 2363): vibehouse already handles all 4 scenarios correctly — `process_withdrawals_gloas` returns early on EMPTY parent without clearing `payload_expected_withdrawals`, envelope validation checks stale withdrawals, block production uses stale value directly. Integration test `gloas_stale_withdrawal_carryover_across_empty_parent` covers this. |
| #4939 | Request missing payload envelopes for index-1 attestation | **Merged 2026-03-24.** Already implemented — envelope request via ExecutionPayloadEnvelopesByRoot RPC when index-1 attestation arrives without envelope. |
| #4979 | PTC window cache in BeaconState | **Merged 2026-03-25.** Proactively implemented — verified (run 2519) implementation matches final merged spec exactly: `compute_ptc`, `get_ptc_committee` (spec `get_ptc`), `process_ptc_window`, `initialize_ptc_window` all match. EF test handler skips schema-mismatched vectors until new test vectors are released with `ptc_window` field. |
| #5040 | Fix fork choice compliance test `is_early_message` bug | **Merged 2026-03-25.** Test infrastructure only — reversed comparison `<` to `>` in Python test generator. No vibehouse code changes needed. |
| #4558 | Cell dissemination via partial message specification | **Merged 2026-03-25.** Adds `PartialDataColumnHeader` container and validation rules for gossipsub partial messages on `data_column_sidecar_{subnet_id}`. Gloas changes mirror Fulu changes (remove `signed_block_header`, add `slot`+`beacon_block_root`). **Not implemented** — requires upstream rust-libp2p gossipsub partial messages support. Vibehouse does not implement Fulu partial messages either. Will implement when libp2p support is available. |
| #5044 | Speed up `compute_ptc` | **Merged 2026-03-26.** Pre-fetches effective_balances before loop. **Already implemented** — vibehouse pre-computes flat indices + effective_balances arrays before the selection loop (gloas.rs:465-477). No code changes needed. |
| #5046 | Increase `compute_shuffled_index` cache from 96 to 65536 | **Merged 2026-03-26.** Python spec tooling only (pysetup/spec_builders/phase0.py). Not relevant to vibehouse. |

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
| ~~#5044~~ | ~~Speed up `compute_ptc`~~ | **MERGED 2026-03-26T19:03:01Z.** Pre-fetches effective_balances, changes `compute_balance_weighted_acceptance` signature. **Already implemented** — vibehouse pre-computes effective_balances (gloas.rs:465-477). No code changes needed. |
| ~~#5046~~ | ~~Increase `compute_shuffled_index` cache~~ | **MERGED 2026-03-26T19:02:36Z.** Python test infra LRU cache optimization. Not relevant to vibehouse. |
| #4843 | Variable PTC deadline | Open, APPROVED. **Proactively implemented** (commit a7baf6b57). |
| #4898 | Remove pending status from tiebreaker | Open — vibehouse already matches post-PR behavior. |
| #4892 | Remove impossible branch in forkchoice | Open — vibehouse already uses debug_assert + ==. |
| #4960 | Fork choice test for new validator deposit | Test vectors — will integrate when released. |
| #4932 | Sanity/blocks tests with payload attestation coverage | Test vectors — will integrate when released. |
| #4954 | Update fork choice store to use milliseconds | Open, 0 reviews, large refactor, also tagged `heze` — not implementing proactively. |
| #4747 | Fast Confirmation Rule | Open, 128+ reviews, CONFLICTING. Actively debated, not close to merge. |
| #4840 | Add support for EIP-7843 to Gloas | Open, stale since Jan 2026. |
| #4630 | EIP-7688: Forward compatible SSZ types | Open, stale since Feb 2026. Not implementing proactively. |

### Consolidated monitoring log (runs 2477-2965, 2026-03-25/26)

**Notable events:**
- Run 2479: #5040 merged (fork choice test bug fix — Python only). #4558 merged, #5041/#5042 merged (Python deps).
- Run 2489: Fixed stale test from #5036 revert.
- Run 2501: Verified #4979 implementation matches final merge (a196ff3e).
- Run 2510: Full health check — 140/140 EF tests, 9/9 fork choice tests pass.
- Run 2547: Audit gap fix — #5001 (bid key 3-tuple) and #5002 (doc-only) confirmed already handled.
- Run 2568: Reviewed #4954 (milliseconds refactor) — large, not implementing proactively.
- Run 2629: Created FCR design doc (`docs/workstreams/fast-confirmation-rule.md`) analyzing PR #4747.
- Run 2805: Updated alloy crates 1.8.1→1.8.2.
- Run 2869: Fixed yanked unicode-segmentation 1.13.1→1.13.2.
- Run 2879: Proactively optimized `compute_ptc` to pre-compute effective balances (matching #5044).
- Run 2922: #5044 merged (compute_ptc speedup) — already implemented. #5046 merged (Python cache) — not relevant.

**Steady state (runs 2971-3047, 2026-03-26/27):**
- No new consensus-specs merges since #5048 (2026-03-27)
- v1.7.0-alpha.4: tag a9bc79a7 exists, no GitHub Release published (latest = v1.7.0-alpha.3)
- EF test vectors: still v1.5.0 (2025-05-07). Upstream nightly generation failing since 2026-03-08. No new test vectors expected until alpha.4 release is published
- CI: fully green (ci, nightly, spec-test-check). Zero clippy/compiler warnings
- cargo audit: 1 rsa vuln in jsonwebtoken transitive dep (no fix available). 5 unmaintained warnings (ansi_term, bincode, derivative, filesystem, paste — all transitive, no action)
- Only outdated root deps require major version bumps: rand 0.9→0.10, reqwest 0.12→0.13, sha2 0.10→0.11, bincode 1→3. None viable as minor updates
- No transitive dep updates available (cargo update --dry-run clean)
- Non-Gloas merges: #5048 (testing label exclusion)
- Non-Gloas open: #5047/#5033 (gossip validation bellatrix/altair), #5045 (remove @always_bls) — none relevant
- Dep updates during this period: cmake 0.1.57→0.1.58, data-encoding-macro-internal syn 1→2, keccak-asm 0.1.6, sha3-asm 0.1.6, simd-adler32 0.3.9, uuid 1.23.0
- #5048 merged 2026-03-27 (testing label exclusion) — not relevant
- All 8 open Gloas PRs unchanged throughout: #4843 (blocked), #4898/#4892 (approved/stale), #4954 (blocked), #4747 (dirty, 146+ review comments), #4960/#4932 (test vectors), #4840/#4630 (stale). #5036 effectively dead.
- Runs 3047-3085 (2026-03-27): Steady state monitoring. No new consensus-specs merges since #5048. v1.7.0-alpha.4 still not published (latest release = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0 (Sep 2025). All deps at latest (0 lockfile changes). CI: all green (ci, nightly, spec-test-check). Zero clippy/compiler warnings. cargo audit unchanged (1 rsa vuln, no fix). Open Gloas PRs unchanged (#4630/#4747/#4840/#4843/#4892/#4898/#4932/#4954/#4960). Non-Gloas PRs: #5049 (capella gossip validation), #5047 (bellatrix gossip validation) — not relevant. No actionable work.
- Runs 3086-3137 (2026-03-27): Steady state. No new consensus-specs merges since #5048. v1.7.0-alpha.4 tag exists but no GitHub Release published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0 (Sep 2025, upstream nightly generation failing since 2026-03-08). CI: all green. Zero clippy warnings. cargo audit: 1 rsa vuln (no fix), 5 unmaintained warnings (transitive). Dep updates during this window: alloy 1.8.2→1.8.3 (run 3119-3121), cc 1.2.57→1.2.58 (run 3125). All other deps at latest (0 lockfile changes). Open Gloas PRs unchanged throughout: #4843 (variable PTC deadline, approved), #4898/#4892 (fork choice simplifications, approved/stale), #4954 (milliseconds, unreviewed), #4747 (FCR, 146+ reviews, conflicts), #4960/#4932 (test vectors), #4840/#4630 (stale). #5036 effectively dead. Non-Gloas: #5049/#5047/#5045 (gossip validation, testing) — not relevant. No actionable spec work.
- Runs 3138-3149 (2026-03-27): Steady state. No new consensus-specs merges since #5048 (testing label exclusion). v1.7.0-alpha.4 tag exists (a9bc79a7), no GitHub Release published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0 (Sep 2025, upstream nightly generation failing since 2026-03-08). CI: all green (ci + nightly). Zero clippy warnings. cargo audit: 1 rsa vuln (no fix), 5 unmaintained (transitive). All deps at latest (0 lockfile changes). All 8 open Gloas PRs unchanged: #4843 (variable PTC deadline, approved), #4898/#4892 (fork choice simplifications, approved/stale), #4954 (milliseconds, unreviewed), #4747 (FCR, 146+ reviews, conflicts), #4960/#4932 (test vectors), #4840/#4630 (stale). Non-Gloas open: #5049 (capella), #5047 (bellatrix), #5033 (altair) gossip validation — not relevant. No actionable work.
- Runs 3150-3154 (2026-03-27): Steady state. No new consensus-specs merges since #5048. v1.7.0-alpha.4 tag exists (a9bc79a7), no GitHub Release published (latest release = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0 (Sep 2025). CI: all green (ci + nightly). Zero clippy warnings. cargo audit unchanged (1 rsa vuln, 5 unmaintained). All deps at latest (0 lockfile changes). All open Gloas PRs unchanged (#4747 FCR updated today but still open/conflicting, 144 reviews). No actionable work.
- Runs 3155-3156 (2026-03-27): Steady state. No new consensus-specs merges since #5048. v1.7.0-alpha.4 tag exists (a9bc79a7), no GitHub Release published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0 (Sep 2025). CI: all green (ci + nightly + spec-test-check). Zero clippy warnings. cargo audit: 1 rsa vuln (no fix), 5 unmaintained (transitive). All deps at latest (0 lockfile changes). All 8 open Gloas PRs unchanged: #4843 (approved), #4898/#4892 (approved/stale), #4954 (unreviewed), #4747 (FCR, updated 2026-03-27 with test commits, still open/conflicting), #4960/#4932 (test vectors), #4840/#4630 (stale). Non-Gloas: #5049/#5047/#5045/#5033 (gossip validation/testing) — not relevant. No actionable work.
- Run 3157 (2026-03-27): Steady state. No new consensus-specs merges since #5048. v1.7.0-alpha.4 tag exists (a9bc79a7), no GitHub Release published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0 (Sep 2025). CI: all green (ci + nightly + spec-test-check). All deps at latest (0 lockfile changes). All 8 open Gloas PRs unchanged. #4747 (FCR) updated today but still open/conflicting. No actionable work.
- Run 3158 (2026-03-27): Steady state. No new consensus-specs merges since #5048. v1.7.0-alpha.4 tag exists (a9bc79a7), no GitHub Release published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0 (Sep 2025). CI: all green. Zero clippy warnings. cargo audit: 1 rsa vuln (no fix). All deps at latest. #4747 (FCR) now at 79 commits, 148 review comments, mergeable_state=dirty. All other open Gloas PRs unchanged. Devnet verified: 4-node finalized epoch 8 (slot 81). No actionable work.
- Run 3159 (2026-03-27): Steady state. No new consensus-specs merges since #5048. v1.7.0-alpha.4 tag exists (a9bc79a7), no GitHub Release published (latest = v1.7.0-alpha.3). EF test vectors still v1.6.0-beta.0 (Sep 2025). CI: all green. Dep update: mio 1.1.1→1.2.0. All open Gloas PRs unchanged. #4747 (FCR) at 79 commits, 148 reviews, still dirty. No actionable spec work.
- Run 3160 (2026-03-27): Steady state. No new consensus-specs merges. No new releases. All deps at latest (0 lockfile changes). CI green. All open Gloas PRs unchanged. No actionable work.
- Run 3161 (2026-03-27): Steady state. No new consensus-specs merges since #5048. No new releases. All deps at latest (0 lockfile changes). No new GitHub issues. All open Gloas PRs unchanged. No actionable work.
- Run 3162 (2026-03-27): Steady state. No new consensus-specs merges since #5048. v1.7.0-alpha.4 tag exists (a9bc79a7), no GitHub Release published (latest = v1.7.0-alpha.3). All deps at latest (0 lockfile changes). CI: all green (ci + nightly + spec-test-check). No new GitHub issues. All open Gloas PRs unchanged. No actionable work.
