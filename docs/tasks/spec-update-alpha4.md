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

## Test Vectors

No v1.7.0-alpha.4 release/tag created yet on consensus-specs (as of run 2358, 2026-03-25). Version bump PR (#5034) merged Mar 24 but no GitHub release published. Spec-test-check workflow will auto-detect when it's published. Current pinned version: v1.7.0-alpha.3.

## Open Gloas PRs to Watch

| PR | Description | Notes |
|----|-------------|-------|
| #4979 | PTC window cache in BeaconState | Major change, renamed to `ptc_window`, still under active design discussion, not merged (as of 2026-03-25) |
| #5036 | Relax bid gossip dependency on proposer preferences | Open — proactively implemented (commit 1c7e608d4) |
| #5035 | Allow same epoch proposer preferences | Open — proactively implemented (commit 3edc6f63d) |
| #4962 | Sanity/blocks tests for missed payload withdrawal interactions | Test vectors |
| #4960 | Fork choice test for new validator deposit | Test vectors |
| #4954 | Update fork choice store to use milliseconds | Open, 0 reviews, large refactor (28 files) — not worth implementing proactively |
| #4932 | Sanity/blocks tests with payload attestation coverage | Test vectors |
| #4898 | Remove pending status from tiebreaker | 1 approval, still open — vibehouse already matches post-PR behavior |
| #4892 | Remove impossible branch in forkchoice | 2 approvals, still open — vibehouse already uses debug_assert + == (matches post-PR) |
| #4747 | Fast Confirmation Rule | Open, actively updated Mar 2026 — new feature, monitor |
| #4843 | Variable PTC deadline | Open, Jan 2026 |
| #4840 | Add support for EIP-7843 to Gloas | Open, Jan 2026 |
