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

## Post-alpha.4 merged PRs (run 2361-2363, 2026-03-25)

| PR | Description | Status |
|----|-------------|--------|
| #5035 | Allow same epoch proposer preferences | **Merged 2026-03-25.** Already implemented — gossip validation accepts current+next epoch (gossip_methods.rs:4111), slot-not-passed check (4124), epoch_offset index calc (4166-4171). VC broadcasts for both epochs (duties_service.rs:1676-1726). |
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

No v1.7.0-alpha.4 release/tag created yet on consensus-specs (as of run 2430, 2026-03-25). Version bump PR (#5034) merged Mar 24 but no GitHub release published. Spec-test-check workflow will auto-detect when it's published. Current pinned version: v1.7.0-alpha.3. EF test vectors also not updated (latest: v1.6.0-beta.0 from Sep 2025). No new Gloas PRs merged since run 2429. All open Gloas PRs unchanged — #4979 (PTC window) still under review (nflaig/jtraglia commenting on naming/docs, no behavioral changes), #4843 (variable PTC deadline) mergeable_state=clean but only COMMENTED reviews (no APPROVED), #4747 (FCR) active design discussion (mkalinin/etan-status debating fallback/reconfirmation behavior). Build and clippy clean (zero warnings). cargo audit unchanged: 1 known vuln (rsa timing side-channel via jsonwebtoken, no fix available), 5 unmaintained warnings in transitive deps. Devnet smoke test passed (run 2430): finalized_epoch=8, clean Gloas fork transition, all 4 nodes healthy through epoch 10. No action needed.

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
