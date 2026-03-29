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
| #5048 | Exclude version file from testing label | **Merged 2026-03-27.** CI/automation only — no code changes. |
| #5051 | Increase test timeout from 12 to 24 hours | **Merged 2026-03-27.** CI infra only — no code changes. |
| #5052 | Reduce `compute_shuffled_index` cache from 65536 to 1024 | **Merged 2026-03-27.** Python spec tooling only (reverts #5046 cache size). Not relevant to vibehouse. |
| #5053 | Rename nightly-tests.yml to tests.yml | **Merged 2026-03-27.** CI file rename only — no code changes. |

### Open Gloas PRs (still monitoring)

| PR | Description | Status |
|----|-------------|--------|
| #5036 | Relax bid gossip dependency on proposer preferences | **Reverted (run 2488)** — PR is effectively dead (both author and reviewer oppose it). Restored spec-compliant behavior: bids are IGNORED when proposer preferences haven't been seen. |
| #4898 | Simplify fork choice is_supporting_vote | Approved, not merged. Already implemented debug_assert. |
| #4892 | Assert slot >= block slot in fork choice | Approved, not merged. Already implemented debug_assert. |
| #4843 | Variable PTC deadline | **Partially implemented** — MIN_PAYLOAD_DUE_BPS config, variable deadline in get_payload_attestation_data. Field rename (payload_present→payload_timely) **reverted** (run 3227) because test vectors use `payload_present` and #4843 hasn't merged. Will re-apply rename when #4843 merges. |
| #4960 | Gloas fork choice test (new validator deposit) | Test vectors — will integrate when released |
| #4932 | Gloas sanity/blocks tests with payload attestation coverage | Test vectors — will integrate when released |
| #4954 | Update fork choice store to use milliseconds | Open. Converts `Store.time`→`Store.time_ms`, `Store.genesis_time`→`Store.genesis_time_ms`. Not merged yet — will implement when merged. |
| #4840 | Add support for EIP-7843 to Gloas | Open (2026-01-15). Raises blob throughput limits. Not merged — will implement when merged. |
| #4630 | EIP-7688: Forward compatible SSZ types in Gloas | Open (2025-10-01). StableContainer/Profile types for light client compatibility. Not merged — will implement when merged. |
| #4747 | Fast Confirmation Rule | Open, 82 commits (latest Mar 25), tests being added. Design doc: `docs/workstreams/fast-confirmation-rule.md`. 6 new Store fields, ~25 functions, Gloas compatibility needed. Will implement when merged. |

## Test Vectors

**v1.7.0-alpha.4 released** (2026-03-27T13:58:28Z). Test vectors downloaded and integrated. Pinned version updated from v1.7.0-alpha.3 to v1.7.0-alpha.4.

**Bug found and fixed (run 3199):** `process_ptc_window` epoch processing test (`process_ptc_window__shifts_all_epochs`) failed because the lookahead epoch (current + MIN_SEED_LOOKAHEAD + 1 = N+2) exceeded `CommitteeCache::initialized`'s epoch bound of N+1. Fix: (1) relaxed CommitteeCache epoch bound from `current_epoch + 1` to `current_epoch + MIN_SEED_LOOKAHEAD + 1` (safe because required RANDAO mix is available), (2) refactored `compute_ptc` into inner/outer functions so `process_ptc_window` can pass an explicit committee cache for the lookahead epoch. All tests passing: 80/80 + 140/140 EF tests, 1033 state_processing, 1085 types, 4998 workspace.

**Field rename fix (run 3227):** Alpha.4 release was re-published with #4979 (PTC window cache) included, meaning test vectors now use `payload_present` for the PayloadAttestationData boolean field. Our proactive #4843 implementation had renamed this to `payload_timely`, causing SSZ static test failures. Reverted to `payload_present` (26 files, 25 Rust source files). Also removed the Gloas test loading skip workaround since alpha.4 vectors now include `ptc_window` field correctly. All tests passing: 80/80 + 140/140 EF tests, 327 fork_choice, 1033 state_processing.

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

### Consolidated monitoring log (runs 2477-3313)

**Notable events:**
- Run 2479: #5040 merged (fork choice test bug fix — Python only). #4558 merged, #5041/#5042 merged (Python deps).
- Run 2489: Fixed stale test from #5036 revert.
- Run 2501: Verified #4979 implementation matches final merge (a196ff3e).
- Run 2547: Audit gap fix — #5001 (bid key 3-tuple) and #5002 (doc-only) confirmed already handled.
- Run 2629: Created FCR design doc (`docs/workstreams/fast-confirmation-rule.md`) analyzing PR #4747.
- Run 2805: Updated alloy crates 1.8.1→1.8.2.
- Run 2869: Fixed yanked unicode-segmentation 1.13.1→1.13.2.
- Run 2879: Proactively optimized `compute_ptc` to pre-compute effective balances (matching #5044).
- Run 2922: #5044 merged (compute_ptc speedup) — already implemented. #5046 merged (Python cache) — not relevant.
- Run 3199: **v1.7.0-alpha.4 released** with test vectors. Fixed CommitteeCache epoch bound for PTC window lookahead. All tests passing: 80/80 + 140/140 EF. Commit 8a83ed8ab.
- Run 3200: Fixed beacon_chain empty-committee panic in `process_ptc_window`. Commit 8181c4647.
- Run 3201: Updated transitive deps (windows-sys, socket2, etc). Commit 71c46c13e.
- Run 3214: Devnet verification — 4-node finalized_epoch=8, clean Gloas fork transition.
- Run 3222: Updated Rust stable 1.94.0→1.94.1 and nightly to 1.96.0-nightly.
- Run 3227: Reverted payload_timely→payload_present to match alpha.4 test vectors.

**Steady state (runs 3202-3315, 2026-03-27 to 2026-03-28):**
- No new consensus-specs merges affecting vibehouse since #5048 (2026-03-27)
- Non-relevant merges: #5051 (test timeout), #5052 (Python cache reduction), #5053 (CI rename)
- v1.7.0-alpha.4 published as GitHub prerelease with test vectors
- CI: all green (ci, nightly, spec-test-check). Zero clippy warnings
- All deps at latest (0 compatible updates). 9 behind latest requiring major bumps (not viable)
- Cargo audit: 1 transitive rsa vuln (no fix available)
- All 8 open Gloas PRs unchanged: #4843 (approved/mergeable), #4898/#4892 (stale), #4954 (unreviewed), #4747 (FCR, 144 reviews, conflicting), #4960/#4932 (test vectors), #4840/#4630 (stale)
- New open PRs (not Gloas): #5055 (EIP-8025 refactor), #5054 (test workflow), #5050 (networking test), #5049/#5047 (gossip validation) — all non-Gloas
- Issue #36: 5 blocked, 2 non-critical remaining
- Rust 1.94.1 stable + 1.96.0-nightly up to date
- Devnet verified multiple times: 4-node finalized_epoch=8, clean Gloas fork transition
- Full test verification (run 3285): 80/80 EF (real crypto) + 140/140 EF (fake crypto) + 2445 core tests + 4998 workspace tests
- Run 3314: Updated transitive deps (wasm-bindgen 0.2.115, js-sys 0.3.92, web-sys 0.3.92). 4998 workspace tests pass. Only outdated dep: rand_xorshift 0.4→0.5 (blocked by rand_core version conflicts)
**Code improvements (runs 3316-3600+, 2026-03-28):**
- Run 3346: Fixed test compilation from ExecutionPayloadBid split (3 test files), fixed cargo doc warning. Tests: beacon_chain 422, network 68, http_api 78 — all passing.
- Run 3502: Added proposer-side bid `inclusion_list_bits` validation per Heze validator.md spec.
- Run 3600: Fixed 3 Heze gossip validation spec compliance gaps: (1) accept previous-slot ILs with attestation_due timing, (2) MAX_BYTES_PER_INCLUSION_LIST=8192 tx size check, (3) committee root mismatch REJECT→IGNORE.

**EF gossip validation tests (runs 3817-3822, 2026-03-28):**
- Run 3817: Added EF tests for `gossip_proposer_slashing` and `gossip_attester_slashing` (all forks phase0→heze). Fixed `is_valid_indexed_attestation` to check validator index range (spec compliance gap). Removed blanket Heze exclusion from `check_all_files_accessed.py`.
- Runs 3819-3820: Added `gossip_beacon_block` (12 phase0 tests), `gossip_beacon_attestation` (16 tests), `gossip_beacon_aggregate_and_proof` (20 tests). All gossip validation test types now implemented.
- Final EF test counts: 86/86 (real crypto) + 148/148 (fake crypto). Zero clippy warnings.

**Heze HTTP API tests (run 3840, 2026-03-29):**
- Added 7 IL duty endpoint tests (`POST /eth/v1/validator/duties/inclusion_list/{epoch}`). 352/352 http_api tests pass.
- No new consensus-specs merges since #5053 (2026-03-27). All open Gloas/Heze PRs unchanged.

**Heze IL slot-1 fix (run 3823, 2026-03-28):**
- Fixed `check_inclusion_list_satisfaction` and `compute_inclusion_list_bits_for_slot` to use `slot - 1` per spec's `record_payload_inclusion_list_satisfaction`. ILs broadcast at slot N-1 constrain the payload at slot N.

**Monitoring steady state (runs 3316-3823+, 2026-03-28/29):**
- No new consensus-specs merges since #5053 (Mar 27). No new releases (still alpha.4).
- Open non-Gloas PRs: #5054 (test workflow), #5055 (EIP-8025 refactor), #5050 (networking test yield fix) — none actionable.
- All monitored Gloas PRs unchanged: #4843 (variable PTC deadline, approved not merged), #4747 (FCR, 148 reviews, conflicting), #4954/#4840/#4630 (stale).
- All open Gloas/Heze PRs unchanged: #4843 (approved/stalled since Mar 20), #4747 (FCR, conflicting, stalled since Feb 16), #4954 (unreviewed), #4898/#4892 (stale), #4960/#4932 (test vectors), #4840/#4630 (stale).
- Deep audits confirmed: all 7 Heze spec documents fully compliant, fork choice IL integration correct, production code unwrap() audit clean.
- Deps fully current. Cargo audit: 1 rsa vuln (no fix). CI all green. Codebase stable.

**Devnet verification (run 3824, 2026-03-29):**
- Heze devnet: Gloas@epoch1, Heze@epoch3, finalized_epoch=8 at slot 81. IL slot-1 fix verified end-to-end.
- Standard Gloas devnet: finalized_epoch=8 at slot 80. Clean fork transition.
- No new consensus-specs merges or releases. All monitored PRs unchanged. Codebase stable.

**Monitoring (run 3825, 2026-03-29):**
- No new consensus-specs merges or releases since alpha.4 (Mar 27). No new Gloas/Heze PRs opened.
- All open Gloas/Heze PRs unchanged: #4843 (approved/stalled), #4747 (FCR, conflicting), #4954/#4898/#4892/#4960/#4932/#4840/#4630 (stale/unreviewed).
- Updated zerocopy 0.8.47→0.8.48. Build clean, zero clippy warnings, cargo audit unchanged (1 rsa vuln).
- EF spec tests: 86/86 (real crypto) + 148/148 (fake crypto). All passing.
- CI green. Deps fully current.

**Monitoring (run 3826, 2026-03-29):**
- No new consensus-specs merges or releases. All monitored Gloas/Heze PRs unchanged.
- Zero clippy warnings. No compatible dependency updates available.
- EF spec tests: 148/148 (fake crypto). Workspace tests: 5065/5065 passing.
- Fixed CLAUDE.md "Before pushing" command to exclude web3signer_tests (requires external Java process, not available in CI/dev environments).

**Heze test coverage + code improvements (runs 3827-3849, 2026-03-29):**
- Added 10 `upgrade_to_heze` unit tests (run 3835), 3 IL gossip validation tests (run 3837), 5 `is_valid_inclusion_list_signature` tests (run 3838), 12 beacon chain integration tests for IL methods (run 3839), 7 IL duty endpoint tests (run 3840), 6 IL pool HTTP API tests (run 3841), 2 block production IL tests (run 3846), 1 fork choice IL test (run 3847). Total: 46 new Heze tests.
- Updated `InclusionListByCommitteeIndices/1` RPC request to match spec: `(slot, Bitvector[INCLUSION_LIST_COMMITTEE_SIZE])` = 10 bytes fixed (run 3842). Fixed gossip validation ordering, RPC handler slot source, response size limit.
- Removed stale `#[allow(dead_code)]`, fixed cargo doc warning for bitvector bracket notation.
- Comprehensive Heze test coverage review (run 3849): 100+ tests across types (29), state processing (21), fork choice (3), beacon chain (14), network (9), validator client (14), HTTP API (13). No gaps found.
- Multiple deep audits: Heze fork choice spec compliance verified, all spec functions accounted for, zero unwrap() in Heze production code, all TODOs have issue links.
- Devnet verified: Gloas 4-node finalized_epoch=8, Heze devnet Gloas@epoch1/Heze@epoch3 finalized_epoch=8.

**Monitoring steady state (runs 3850-3871, 2026-03-29):**
- No new consensus-specs merges or releases since alpha.4 (Mar 27). No new Gloas/Heze PRs opened.
- All open Gloas/Heze PRs unchanged: #4843 (approved/stalled since Mar 20), #4747 (FCR, REVIEW_REQUIRED, conflicting, 148 reviews), #4954/#4898/#4892/#4960/#4932/#4840/#4630 (stale/unreviewed).
- Non-Gloas PRs: #5055 (EIP-8025 refactor), #5054 (test workflow), #5050 (networking test yield), #5049/#5047 (gossip validation for older forks) — none actionable.
- Updated transitive dep: syn 1.0.109→2.0.117 (run 3865).
- Full TODO audit (run 3866): all remaining TODOs blocked on external deps (EIP-7892 ×3, blst safe API, PeerDAS checkpoint sync) or non-critical (#36). No unimplemented!() in production code.
- Full Heze audit (run 3871): zero TODOs/FIXMEs/unimplemented!() in all Heze/FOCIL code. All 7 implementation phases verified clean.
- Zero clippy warnings. No compatible dependency updates. No transitive dep updates. Cargo audit: 1 rsa vuln (no fix). CI green. EF spec tests: 86/86 + 148/148. Devnet verified: finalized_epoch=8. Codebase stable.

**Monitoring (runs 3872-3875, 2026-03-29):**
- No new consensus-specs merges or releases since alpha.4 (Mar 27). No new Gloas/Heze PRs opened.
- All 9 monitored open Gloas/Heze PRs unchanged: #4843 (approved/stalled since Mar 20), #4747 (FCR, conflicting, 148 review comments), #4954 (unreviewed), #4898/#4892 (approved but stalled), #4960/#4932 (test vectors, #4932 conflicting), #4840/#4630 (stale).
- Non-Gloas PRs: #5054 (test workflow), #5055 (EIP-8025 refactor), #5049/#5047/#5033 (gossip validation tests for capella/bellatrix/altair) — none actionable.
- Zero clippy warnings. No compatible dependency updates. Cargo audit: 1 transitive rsa vuln (no fix). CI green. Codebase stable.

**Monitoring (runs 3876-3877, 2026-03-29):**
- No new consensus-specs merges or releases since alpha.4 (Mar 27). Last merge: #5053 (CI rename, Mar 27).
- All 9 monitored open Gloas/Heze PRs unchanged: #4843 (approved/stalled), #4747 (FCR, conflicting), #4954 (unreviewed), #4898/#4892 (stalled), #4960/#4932 (test vectors), #4840/#4630 (stale).
- Non-Gloas PRs: #5054 (test workflow), #5055/#5050/#5049/#5047/#5033 — none actionable.
- Zero clippy warnings. No dependency updates. Cargo audit: 1 rsa vuln (no fix). CI green. Codebase stable.
