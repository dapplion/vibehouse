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
| #5036 | Relax bid gossip dependency on proposer preferences | Open — proactively implemented. |
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
