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
| #5054 | Update tests workflow | **Merged 2026-03-29.** CI workflow update — no code changes. |

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

## Open Gloas PRs to Watch

| PR | Description | Notes |
|----|-------------|-------|
| ~~#4979~~ | ~~PTC window cache in BeaconState~~ | **MERGED 2026-03-25.** Proactively implemented and verified against final merge commit (a196ff3e). |
| ~~#5035~~ | ~~Allow same epoch proposer preferences~~ | **MERGED 2026-03-25.** Already implemented — no code changes needed. |
| ~~#4558~~ | ~~Cell Dissemination via Partial Messages~~ | **MERGED 2026-03-25.** Not actionable — requires Gossipsub partial messages extension (no Rust libp2p implementation). |
| ~~#5036~~ | ~~Relax bid gossip dependency on proposer preferences~~ | **Effectively dead** — both author and reviewer opposed. Proactive implementation reverted. |
| ~~#5044~~ | ~~Speed up `compute_ptc`~~ | **MERGED 2026-03-26.** Already implemented — vibehouse pre-computes effective_balances. |
| ~~#5046~~ | ~~Increase `compute_shuffled_index` cache~~ | **MERGED 2026-03-26.** Python test infra only. Not relevant. |
| #5056 | Add check on bid gossip for blob kzg commitment len | **Open 2026-03-29.** Approved (2 reviews), not merged. Proactively implemented — commit 42e33200c. |
| #4843 | Variable PTC deadline | Open, APPROVED. **Proactively implemented** (commit a7baf6b57). Field rename reverted pending merge. |
| #4898 | Remove pending status from tiebreaker | Open — vibehouse already matches post-PR behavior. |
| #4892 | Remove impossible branch in forkchoice | Open — vibehouse already uses debug_assert + ==. |
| #4960 | Fork choice test for new validator deposit | Test vectors — will integrate when released. |
| #4932 | Sanity/blocks tests with payload attestation coverage | Test vectors — will integrate when released. |
| #4954 | Update fork choice store to use milliseconds | Open, 0 reviews, large refactor, also tagged `heze` — not implementing proactively. |
| #4747 | Fast Confirmation Rule | Open, 148+ reviews, CONFLICTING. Design doc ready (`docs/workstreams/fast-confirmation-rule.md`). |
| #4840 | Add support for EIP-7843 to Gloas | Open, stale since Jan 2026. |
| #4630 | EIP-7688: Forward compatible SSZ types | Open, stale since Feb 2026. Not implementing proactively. |

## Consolidated Monitoring Log (runs 2477-4100)

### Key events

- **#4979 merged** (2026-03-25): proactive implementation verified against final merge (a196ff3e). No changes needed.
- **#5035 merged** (2026-03-24): already implemented (same-epoch proposer preferences).
- **#5040 merged** (2026-03-25): fork choice test bug fix — Python only.
- **#4558 merged** (2026-03-25): partial messages — blocked on rust-libp2p support.
- **#5044 merged** (2026-03-26): compute_ptc speedup — already implemented.
- **v1.7.0-alpha.4 released** (2026-03-27): test vectors downloaded, all tests passing 86/86 + 148/148 EF.
- **#5054 merged** (2026-03-29): test workflow update — CI only.
- Created FCR design doc (`docs/workstreams/fast-confirmation-rule.md`) analyzing PR #4747.
- Issue #5043 audited (Gloas genesis block hash): vibehouse already handles correctly.

### Bug fixes shipped

- Fixed CommitteeCache epoch bound for PTC window lookahead (commit 8a83ed8ab).
- Fixed beacon_chain empty-committee panic in `process_ptc_window` (commit 8181c4647).
- Reverted payload_timely→payload_present to match alpha.4 test vectors.
- Fixed HTTP API `post_beacon_pool_proposer_preferences` missing epoch/slot validation.
- Fixed `check_inclusion_list_satisfaction` slot-1 per spec (ILs at slot N-1 constrain payload at slot N).
- Fixed missing InclusionListStore pruning — unbounded memory growth (commit 264469b7d).
- Fixed `is_valid_indexed_attestation` validator index range check (spec compliance gap).
- Fixed 3 Heze gossip validation spec compliance gaps.
- Replaced 2 production `unreachable!()` with proper error handling.

### Code improvements shipped

- Proactively implemented #5056 (blob kzg commitment length check, commit 42e33200c).
- Optimized: `compute_ptc_inner` (eliminated allocations), `get_missing_columns_for_epoch` (direct slice indexing), `handle_data_columns_by_root_request` (iter().copied()), Heze IL processing (removed clones).
- Added VC-side Prometheus metrics for payload attestation + inclusion list services.
- Removed dead code enum variants, stale `#[allow(dead_code)]`.
- Updated `InclusionListByCommitteeIndices/1` RPC to match spec (10 bytes fixed).
- Added proposer-side bid `inclusion_list_bits` validation per Heze validator.md spec.

### Test coverage added

- 46 Heze tests (upgrade, gossip, signatures, beacon chain, HTTP API, block production, fork choice).
- EF gossip validation tests: proposer/attester slashings, beacon blocks (12), attestations (16), aggregates (20).
- 34 SingleLookupRequestState/SingleBlockLookup state machine tests.
- 23 sync RPC request handling tests.
- 14 ColumnRequest custody state machine tests.
- 16 blob/data column request items validation tests.
- 11 edge case tests (get_best_execution_bid IL bits, get_ptc_committee epoch bounds, InclusionListStore::prune).
- Final EF test counts: 86/86 (real crypto) + 148/148 (fake crypto).

### Audits completed

- ~95-98% of Gloas/Heze public functions tested (217 unit + 89 integration tests).
- Zero production unwrap/todo!/FIXME/HACK. All TODOs linked to issues.
- All remaining untested code requires BeaconChain<T>/BeaconState integration harnesses.
- All `unreachable!()` safe by construction, all `unimplemented!()` are test-only mocks.

### ROCQ formal proofs (run 4087)

- Added fork choice proofs (tier 1): 30 theorems/lemmas covering head selection, pruning safety, Gloas 3-state payload model, reorg resistance.

### Steady state (runs 3966-4100, 2026-03-29/30)

No new consensus-specs merges or releases since alpha.4 (Mar 27). All open Gloas/Heze PRs unchanged: #4843 (approved/stalled since Mar 20), #4747 (FCR, conflicting), #5056 (approved, not merged), #4954/#4898/#4892/#4960/#4932/#4840/#4630 (stale/unreviewed). Zero clippy warnings. Toolchains: stable 1.94.1, nightly 1.96.0. Cargo audit: 1 rsa vuln (no fix). EF tests: 86/86 + 148/148. CI all green. Codebase stable.
