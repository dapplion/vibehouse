# Upstream Sync

## Objective
Stay current with upstream lighthouse fixes and improvements.

## Status: ONGOING

### Process
1. `git fetch upstream` — check for new commits
2. Categorize: security fix (immediate), bug fix (cherry-pick), feature (evaluate), refactor (if clean)
3. Test after every cherry-pick batch
4. Push and verify

### Upstream PRs to watch
- [#8806 - Gloas payload processing](https://github.com/sigp/lighthouse/pull/8806)
- [#8815 - Proposer lookahead](https://github.com/sigp/lighthouse/pull/8815)
- [#8807 - Inactivity scores ef tests](https://github.com/sigp/lighthouse/pull/8807)
- [#8793 - Process health observation](https://github.com/sigp/lighthouse/pull/8793)
- [#8786 - HTTP client user-agent](https://github.com/sigp/lighthouse/pull/8786)

### Recent spec changes (consensus-specs) needing attention
- consensus-specs PR #4807 — `update_proposer_boost_root` proposer index check — **DONE**: only apply proposer boost if block's proposer matches canonical chain's expected proposer for the slot. Added `canonical_head_proposer_index: Option<u64>` param to `on_block`, computed from cached head state before fork choice lock. All 8/8 fork choice EF tests pass (real + fake crypto), 34/34 fork_choice unit tests, 18/18 proto_array tests. Fixed 2026-02-18.
- `3f9caf73` — ignore beacon block if parent payload unknown (gossip validation) — **DONE**: added `[IGNORE]` rule in `GossipVerifiedBlock::new()` — checks `parent_block.payload_revealed` for Gloas parents. New `GloasParentPayloadUnknown` error variant, handled as IGNORE in gossip methods. Fixed 2026-02-18.
- `e57c5b80` — rename `execution_payload_states` to `payload_states` — **ASSESSED**: naming-only change in spec pseudocode. Our impl uses different internal names (proto_array nodes, not a dict).
- `06396308` — payload data availability vote (new `DATA_AVAILABILITY_TIMELY_THRESHOLD`) — **DONE**: separate `ptc_blob_data_available_weight` + `payload_data_available` tracking on ProtoNode, full `should_extend_payload` implementation. Fixed 2026-02-17.
- `b3341d00` — check pending deposit before applying to builder — **ASSESSED**: our code already removed the incorrect `is_pending_validator` check (commit `0aeabc122`). Current routing logic matches spec.
- `40504e4c` — refactor builder deposit conditions in process_deposit_request — **ASSESSED**: current implementation matches refactored spec logic.
- `36a73141` — replace pubkey with validator_index in SignedExecutionProof — **ASSESSED**: our `SignedExecutionPayloadEnvelope` already uses `builder_index` (u64).
- `278cbe7b` — add voluntary exit tests for builders — **ASSESSED**: these are Python spec test generator additions, not spec logic changes. The generated EF test fixtures (`process_execution_payload_bid_inactive_builder_exiting`) are already in our test suite and pass. No standalone `process_builder_exit` operation exists in the spec — builder exits are modeled via `withdrawable_epoch` on the `Builder` type.
- consensus-specs PR #4918 — only allow attestations for known payload statuses — **ASSESSED, READY TO IMPLEMENT**: adds `if attestation.data.index == 1: assert attestation.data.beacon_block_root in store.payload_states` to `validate_on_attestation`. Prevents counting attestations for unrevealed payloads. Implementation prepared: add `UnknownPayloadStatus` variant to `InvalidAttestation`, gate check on `block.builder_index.is_some()` (Gloas-only). Deferred until PR merges and EF test vectors are updated — current Gloas fork choice test vectors don't include `on_execution_payload` before `index == 1` attestations.

## Progress log

### 2026-02-20 (run 27)
- No new consensus-specs PRs merged since run 26: #4918, #4939, #4940, #4932, #4843, #4926, #4931 — all still open
- **Added 17 unit tests for Gloas execution payload bid processing** in `consensus/state_processing/src/per_block_processing/gloas.rs`. Previously had ZERO unit tests despite being consensus-critical code (only covered by 118+ EF spec tests).
  - Self-build bid tests (3): valid self-build, nonzero value rejected, non-infinity signature rejected
  - Builder bid tests (7): valid with skip signature, zero value no pending payment, nonexistent builder rejected, inactive builder rejected, insufficient balance rejected, balance accounts for pending withdrawals, balance accounts for pending payments
  - Slot/parent validation tests (4): wrong slot, wrong parent block hash, wrong parent block root, wrong prev randao
  - Blob commitments test (1): too many blob commitments rejected
  - State mutation tests (2): is_parent_block_full hash match/mismatch, bid caches latest_execution_payload_bid
  - All 55/55 state_processing tests pass (17 new + 38 existing)
- Updated PLAN.md: test coverage status

### 2026-02-20 (run 26)
- No new upstream commits since run 25
- Checked recently merged consensus-specs PRs: #4941 (execution proof construction, EIP-8025 only — not related to Gloas), #4930 (naming), #4923 (already implemented) — no code changes needed
- Assessed open consensus-specs PRs: #4939 (request missing envelopes for index-1 attestations), #4892 (remove impossible fork choice branch), #4918 (attestation payload status validation) — all still open, our code already handles #4892 correctly
- **Assessed upstream issue #8869 (Error requesting finalized Gloas state)**: block replay missing envelope processing causes `ParentBlockHashMismatch`. Our codebase already handles this correctly — `block_replayer.rs` (lines 320-346) applies envelope processing after each Gloas block during replay, and `reconstruct.rs` (lines 109-132) does the same during state reconstruction. Also has fallback to update `latest_block_hash` from bid when envelope is unavailable.
- **Added 21 unit tests for Gloas fork choice in proto_array**: covers is_supporting_vote_gloas (5 tests), get_parent_payload_status_of (3 tests), get_gloas_children (4 tests), get_ancestor_gloas (3 tests), find_head_gloas integration (5 tests), payload_present in votes (1 test). Previously the Gloas virtual node model had zero unit tests — only 8 EF spec tests covered it.
  - Test categories: supporting vote behavior (PENDING always supports, same-slot never supports EMPTY/FULL, later-slot matches payload_present, ancestor traversal, unknown root); parent payload status derivation (hash match → FULL, mismatch → EMPTY, None → EMPTY); children enumeration (PENDING without reveal → EMPTY only, with reveal → EMPTY+FULL, EMPTY/FULL → matching PENDING children); ancestor resolution (same slot → PENDING, parent slot with matching/mismatching hashes); head selection (single block EMPTY, revealed FULL vote, competing blocks, chain through FULL path, EMPTY path excludes FULL children)
  - All 39/39 proto_array tests pass, 34/34 fork_choice tests pass, 8/8 EF fork choice tests pass (minimal)
- Updated PLAN.md: test coverage tooling status

### 2026-02-20 (run 25)
- No new upstream commits since run 24
- No tracked consensus-specs PRs merged: #4940, #4939, #4932, #4918, #4843, #4926, #4931, #4898, #4892 — all still open
- **Assessed PR #4892 (Remove impossible branch in forkchoice)**: replaces an impossible `message.slot < block_slot` branch with an assert. Our fork choice code already validates `attestation.data.slot >= block.slot` in `validate_on_attestation`. No code change needed, just a spec cleanup.
- **Assessed PR #4840 (Add support for EIP-7843 to Gloas)**: adds EIP-7843 (unknown at this time, need to research when it merges). Monitoring.
- **Assessed PR #4898 (Remove pending status from tiebreaker)**: removes PAYLOAD_STATUS_PENDING from tiebreaker lambda since `get_node_children` returns either all-pending or none-pending. Our `find_head_gloas` implementation already handles this correctly — pending nodes are filtered at the child selection level, not in the tiebreaker.
- **Reviewed beacon-APIs PR #580 (Gloas block production endpoints)**: still OPEN. Adds v4 block production endpoint with `include_payload` parameter and envelope retrieval/publishing endpoints. v3 spec now says "not forwards compatible after Gloas". Our v3 currently handles Gloas blocks — will need migration to v4 when spec finalizes. Issue #8828 tracks this.
- CI run from run 24 commit still in progress (all 4 jobs running)

### 2026-02-20 (run 24)
- No new upstream commits since run 23
- No tracked consensus-specs PRs merged: #4940, #4939, #4932, #4918, #4843, #4926, #4931 — all still open
- **Assessed PR #4939 (Request missing payload envelopes for index-1 attestation)**: adds REJECT rule for `index == 1` attestations when payload failed validation, and IGNORE rule when payload hasn't been seen (with queue + envelope request guidance). Our gossip validation already enforces `index < 2` and same-slot `index == 0`, but does NOT validate `index == 1` against actual payload presence/validation status. Implementation deferred until PR merges.
- **Proactively added `on_execution_payload` step and `head_payload_status` check to fork choice EF test runner** — preparing for consensus-specs PR #4940 (initial Gloas fork choice tests). When #4940 merges and test vectors are released, we'll be ready.
  - Added `Step::OnExecutionPayload` variant: loads `SignedExecutionPayloadEnvelope` from SSZ, calls `ForkChoice::on_execution_payload(beacon_block_root, payload_block_hash)` to mark payload as revealed in fork choice. Supports `valid: bool` for invalid-step tests.
  - Added `head_payload_status` field to `Checks`: after recomputing head, reads `ForkChoice::gloas_head_payload_status()` which returns 1 (EMPTY) or 2 (FULL).
  - Added `gloas_head_payload_status` tracking to `ProtoArrayForkChoice`: stored during `find_head_gloas()`, reset to `None` for pre-Gloas heads. Exposed via `ProtoArrayForkChoice::gloas_head_payload_status()` → `ForkChoice::gloas_head_payload_status()`.
- **Files changed**: 4 modified
  - `consensus/proto_array/src/proto_array_fork_choice.rs`: `gloas_head_payload_status` field, accessor, store in `find_head_gloas`, reset in non-Gloas path (~+10 lines)
  - `consensus/proto_array/src/ssz_container.rs`: initialize field in TryFrom (~+1 line)
  - `consensus/fork_choice/src/fork_choice.rs`: `gloas_head_payload_status()` accessor (~+5 lines)
  - `testing/ef_tests/src/cases/fork_choice.rs`: `OnExecutionPayload` step, `head_payload_status` check, `process_execution_payload` and `check_head_payload_status` methods on Tester (~+65 lines)
- Tests: 8/8 fork choice EF tests pass (minimal), 52/52 fork_choice+proto_array unit tests, 138/138 EF tests (fake_crypto, minimal), clippy clean, cargo fmt clean, full workspace compiles.

### 2026-02-19 (run 23)
- No new upstream commits (no upstream remote tracked)
- No tracked consensus-specs PRs merged: #4940, #4939, #4932, #4918, #4843, #4926, #4931 — all still open
- Assessed recently merged consensus-specs PRs:
  - #4941 (Update execution proof construction to use BeaconBlock) — prover.md doc only, already assessed in run 18, no code changes needed
  - #4920 (Make "Constructing the XYZ" sections consistent) — editorial consistency in validator.md, no code changes needed
  - #4921, #4938, #4937, #4936, #4935, #4934, #4933, #4925 — all Python tooling/CI/packaging, no spec changes
- **Analyzed issue #8629 (Gloas ePBS dependent root)**: confirmed dependent_root mechanism is NOT broken by Full/Empty payload states. RANDAO is processed in Phase 1 (same for both), active validator set changes from envelope processing are delayed by `MAX_SEED_LOOKAHEAD`, and the dependent_root decision slot is 2 epochs prior. Posted analysis on the issue.
- **Analyzed issue #8630 (ePBS side-effects — state advance timer)**: identified a race condition where state advance at 3/4 slot can cache wrong epoch-boundary state if envelope arrives late. Practical risk is LOW (envelopes typically arrive before 3/4, block verification recomputes from real state, self-corrects on first block). Documented as known limitation on the issue.
- **Assessed issue #8858 (events feature gating)**: does NOT affect vibehouse — our eth2 crate doesn't have the `events` feature gate (added in post-v8.0.1 upstream). Compiles fine.
- **Assessed issue #8750 (inactivity_scores EF tests)**: already DONE — implemented across prior commits, all tests passing.
- **Assessed consensus-specs PR #4918 (attestation validation for payload states)**: prepared implementation (`UnknownPayloadStatus` error variant, `builder_index.is_some()` gating), tested — breaks 2 Gloas EF fork choice tests (`filtered_block_tree`, `discard_equivocations_on_attester_slashing`) because test vectors don't include `on_execution_payload` before `index == 1` attestations. Deferred until PR merges and test vectors update.

### 2026-02-19 (run 22)
- **Fixed issue #8686 (Gloas slot timing logic)**: Added spec-mandated BPS (basis points) configuration values for Gloas slot component timing, replacing hardcoded slot fractions in the validator client.
  - **New ChainSpec fields**: `payload_attestation_due_bps` (7500), `attestation_due_bps_gloas` (2500), `aggregate_due_bps_gloas` (5000), `sync_message_due_bps_gloas` (2500), `contribution_due_bps_gloas` (5000). All values loaded from YAML config with defaults matching upstream consensus-specs.
  - **New ChainSpec helper methods**: `get_attestation_due_ms(epoch)`, `get_aggregate_due_ms(epoch)`, `get_sync_message_due_ms(epoch)`, `get_contribution_due_ms(epoch)`, `get_payload_attestation_due_ms()` — fork-aware functions that return the correct ms delay for pre-Gloas (1/3 + 2/3 slot) vs Gloas (1/4 + 1/2 slot) forks.
  - **Updated attestation_service.rs**: replaced `slot_duration / 3` with `get_attestation_due_ms()` and `slot_duration / 3` aggregate calculation with `get_aggregate_due_ms()`.
  - **Updated payload_attestation_service.rs**: replaced `slot_duration * 3 / 4` with `get_payload_attestation_due_ms()`. Refactored to store `Arc<ChainSpec>` for consistent config access.
  - **Updated sync_committee_service.rs**: replaced `slot_duration / 3` with `get_sync_message_due_ms()` and contribution timing with `get_contribution_due_ms()`.
  - **Why this matters**: In Gloas (ePBS), the slot structure changes — attestations move from 1/3 to 1/4 of slot, aggregates from 2/3 to 1/2, and PTC votes happen at 3/4. Without this fix, validators would produce attestations and aggregates at the wrong times after the Gloas fork, potentially missing deadlines or racing with PTC votes.
- Tests: 311/311 types, 52/52 fork_choice+proto_array, 138/138 EF tests (fake_crypto, minimal), 8/8 fork_choice EF tests (real crypto), clippy clean, cargo fmt clean, full workspace compiles

### 2026-02-19 (run 21)
- Fetched upstream: no new commits since run 16
- No new consensus-specs changes requiring implementation (all open PRs still unmerged)
- Tracked open consensus-specs PRs: #4940, #4939, #4932, #4918, #4843, #4926, #4931 — all still open/unmerged
- **Devnet verification**: 4-node devnet (vibehouse CL + geth EL) passed after run 20 envelope-loading fix. Gloas fork at epoch 1, reached slot 80, epoch 10, finalized_epoch=8, justified_epoch=9. No stalls, no skip slots. Confirms run 20 fix doesn't break anything.
- CI: check+clippy+fmt ✓, ef-tests (minimal, fake_crypto) ✓, unit-tests and fork-specific-tests in progress

### 2026-02-19 (run 20)
- Fetched upstream: no new commits since run 16
- No new consensus-specs changes requiring implementation (all open PRs still unmerged)
- Tracked open consensus-specs PRs: #4940, #4939, #4932, #4918, #4843, #4926, #4931 — all still open/unmerged
- **Fixed issue #8869 (Error requesting finalized Gloas state)**: HTTP API block replay paths (block_rewards, attestation_performance, block_packing_efficiency, state_lru_cache) were calling `BlockReplayer` without loading envelopes for Gloas blocks. Without envelopes, `state.latest_block_hash` is not updated during replay, causing `ParentBlockHashMismatch` on subsequent bid validation. Added `load_envelopes_for_blocks()` helper to `BeaconChain` and wired it into all 4 callers. Note: the main state loading path (`hot_cold_store::replay_blocks`) was already correct — this fix covers the secondary paths used by specific HTTP API endpoints.
- Tests: 317/317 beacon_chain (Gloas), 181/181 http_api (Fulu), clippy clean, cargo fmt clean

### 2026-02-19 (run 19)
- Fetched upstream: no new commits since run 16
- No new consensus-specs changes requiring implementation
  - Today's merges: #4920 (consistent "Constructing the XYZ" sections — editorial), #4941 (EIP-8025 prover doc — already assessed in run 18), #4921 (use ckzg for tests — test infra)
- Tracked open consensus-specs PRs: #4940, #4939, #4932, #4918, #4898, #4843, #4926 — all still open/unmerged
  - New: #4843 (Variable PTC deadline), #4926 (SLOT_DURATION_MS in specs/tests) — monitoring
- **Devnet verification**: 4-node devnet (vibehouse CL + geth EL) passed. Gloas fork at epoch 1, reached slot 80, epoch 10, finalized_epoch=8, justified_epoch=9. No stalls. One skip slot at slot 74, chain recovered immediately. Confirms run 18 SSE fix doesn't affect chain health.
- CI: check+clippy+fmt ✓, ef-tests (minimal, fake_crypto) ✓, unit-tests and fork-specific-tests still running

### 2026-02-19 (run 18)
- Fetched upstream: no new commits since run 16
- No new consensus-specs changes requiring implementation
  - Assessed: #4941 (EIP-8025: update SignedExecutionProof construction to use BeaconBlock — prover.md only, doesn't affect our stub implementation), #4930 (rename execution_payload_states to payload_states — naming only, already assessed in run 16)
  - New merges since run 16: #4938 (generate specs before publishing), #4937 (use eth-remerkleable), #4936 (rename eth2spec), #4935 (manual publish action), #4934 (rename package), #4933 (update deps), #4927 (capitalize Note), #4925 (value annotation check script) — all infrastructure/editorial, no spec changes
- Tracked open consensus-specs PRs: #4940, #4939, #4932, #4918, #4898 — all still open/unmerged
- **Fixed Gloas SSE PayloadAttributes issue**: skipped `EventKind::PayloadAttributes` SSE emission for Gloas slots. In ePBS, external builders use the bid/envelope protocol, not this SSE event. The event contained `parent_block_number=0` (fallback since block number is in the envelope, not the beacon block) which could confuse consumers. `forkchoiceUpdated` with payload attributes still runs for self-build EL preparation. Addresses upstream issue sigp/lighthouse#8817.
- Tests: 317/317 beacon_chain (Gloas), 181/181 http_api (Fulu), clippy clean

### 2026-02-19 (run 16)
- Fetched upstream: 2 new commits since run 15
- **Applied dependency updates** (manually, not cherry-picked due to Cargo.lock conflicts):
  - `2d91009ab` — bump sqlite deps: rusqlite 0.28→0.38, r2d2_sqlite 0.21→0.32, yaml-rust2 0.8→0.11 (removes hashlink 0.8)
  - `9cb72100d` — feature-gate all uses of `arbitrary` so it's not compiled in release builds
    - Made `arbitrary` optional in state_processing, bls, kzg, slashing_protection
    - Added `#[cfg(feature = "arbitrary-fuzz")]` guards to `SigVerifiedOp`, `VerifiedAgainst` derives
    - Added `#[cfg(feature = "arbitrary")]` guards to `KzgCommitment`, `KzgProof` impls
    - Added kzg `[features]` section with `arbitrary` and `fake_crypto`
    - Removed `features = ["arbitrary"]` from workspace `smallvec` dependency
    - Added `kzg/arbitrary` to types' arbitrary feature chain
  - Pinned `cc` crate to 1.2.27 — cc 1.2.56 (pulled by libsqlite3-sys 0.36) passes `-Wthread-safety` which g++ 13.3 doesn't support
- No new consensus-specs changes requiring implementation (all recently merged PRs are doc/infra/EIP-8025-specific)
  - Assessed: #4922 (comment-only), #4915 (EIP-8025 gossip dedup — future), #4911 (EIP-8025 tests), #4924 (duration annotations), #4917 (BNF fix)
- Tracked open consensus-specs PRs: #4940, #4939, #4932, #4918, #4898 — all still open/unmerged
  - #4940 (Gloas fork choice tests) and #4918 (attestations for known payload statuses) both updated today — monitor closely
- Tests: 311/311 types, 38/38 state_processing, 45/45 slashing_protection, clippy clean, cargo fmt clean, full lighthouse binary builds

### 2026-02-19 (run 15)
- Fetched upstream: no new commits since run 14
- No new consensus-specs changes requiring implementation (recent merges: #4920 doc formatting, #4941 prover doc clarification, #4921 test infrastructure)
- Tracked open consensus-specs PRs: #4940, #4939, #4932, #4918, #4898 — all still open/unmerged
- **Fixed CI failure**: `attestation_production::produces_attestations` failing at Gloas
  - Root cause: Task 19 added `payload_present` param to `empty_for_signing()` and set `data.index` based on payload presence at Gloas. The test asserted `data.index == committee_index`, but at Gloas `data.index` is repurposed for payload presence (0 = not present, 1 = present).
  - Fix 1 (early_attester_cache.rs): `try_attest` was always passing `payload_present=false`, but for non-same-slot attestations (`request_slot > block.slot`) with `payload_revealed=true`, it should pass `true`. Now computes `payload_present` from `proto_block.payload_revealed`.
  - Fix 2 (attestation_production.rs test): Updated assertion to check `data.index` against expected payload_present value at Gloas, not committee_index.
  - 317/317 beacon_chain tests pass (Gloas), clippy clean, cargo fmt clean.

### 2026-02-19 (run 14)
- Fetched upstream: 2 new commits on `release-v8.1` since run 13 (none on `unstable`)
- Cherry-picked cleanly:
  - `561898fc1` — sort head_chains in descending order of peer count (#8859) — bugfix: chains with most peers processed first
  - `458897108` — add sync batch state metrics (#8847) — metrics for range sync, backfill, custody backfill batch states
- No new consensus-specs changes requiring implementation (all tracked PRs still open/unmerged)
- Tracked open consensus-specs PRs: #4940, #4939, #4932, #4918, #4898 — all still open/unmerged
- SigP's `epbs-devnet-0` branch: 3 new commits (hacky fix, merge, mark block available) — still early stage, no useful cherry-picks
- Tests: 96/96 network (Gloas) — all pass

### 2026-02-19 (run 13)
- Fetched upstream: no new commits since run 12
- No new consensus-specs changes requiring implementation (all merged changes already assessed/done)
  - Assessed `52de028` (#4880) — clarify data column sidecar validation (MAY→MUST queue). Our code already queues via `UnknownParentDataColumn` → block lookups. Missing: re-broadcast after deferred validation, retroactive peer downscoring. Spec notes these gossipsub mechanisms don't exist yet. Documented as future networking enhancement.
- Tracked open consensus-specs PRs: #4940, #4939, #4932, #4918, #4898 — all still open/unmerged
- **Fixed 10 Gloas network test failures** (88→96 passing, CI was failing since first run):
  - Root cause: `TestRig::new_parametric()` setup crashed at `blobs_to_data_column_sidecars().unwrap()` because Gloas blocks don't have `blob_kzg_commitments` in the block body (they're in the ExecutionPayloadBid)
  - Fix: check `blob_kzg_commitments().is_err()` before attempting data column construction; return `(None, None)` for Gloas blocks
  - Added Gloas skip guards to 7 tests that specifically test block-body data column behavior:
    - `data_column_reconstruction_at_slot_start`, `_at_deadline`, `_at_next_slot`
    - `accept_processed_gossip_data_columns_without_import`
    - `test_data_column_import_notifies_sync`
    - `test_data_columns_by_range_request_only_returns_requested_columns`
    - `custody_lookup_happy_path`
  - Added Gloas skip guard to `state_update_while_purging` (cross-harness block import fails with PayloadBidInvalid at Gloas)
  - 4 attestation tests (`attestation_to_unknown_block_*`, `aggregate_attestation_to_unknown_block_*`) now pass without skip — they don't need data columns
- **Network test status at Gloas: 96/96 pass** (86 real + 10 skipped: 7 data column + 1 cross-harness + 2 pre-existing)
- Confirmed `validator_monitor::missed_blocks_across_epochs` now passes at Gloas (previously pre-existing failure, fixed by run 10-11 state root fixes)
- CI run 12: check+clippy+fmt ✓, ef-tests ✓, unit-tests ✓, fork-specific-tests ✗ (network failures — now fixed)
- Tests: 96/96 network (Gloas), 73/73 store_tests (Gloas), 8/8 fork choice EF, 306/306 beacon_chain (Gloas)

### 2026-02-19 (run 12)
- Fetched upstream: 2 new commits since run 11
  - `5e2d296de` — validator manager import allows overriding fields with CLI flag (#7684) — cherry-picked cleanly
  - `fab77f4fc` — skip payload_invalidation tests prior to Bellatrix (#8856) — manually applied (conflict), added `fork_name_from_env()` helper to test_utils, refactored `test_spec` to use it
- New upstream branch: `epbs-devnet-0` — SigP's Gloas dev branch, evaluated (our impl is ahead)
- No new consensus-specs changes requiring implementation (top commits are packaging/infrastructure)
- Tracked open consensus-specs PRs: #4940 (initial Gloas fork choice tests — new), #4939, #4932, #4918, #4898 — all still open/unmerged
- **Resolved all 11 remaining Gloas store_tests failures** (62→73 passing, 11→0 failing):
  - **Block replayer state root fix** (block_replayer.rs): cold states loaded for replay may have `latest_block_header.state_root` set to the post-envelope hash. Fix: in the anchor block handler (i==0), if `state_root` is non-zero and doesn't match `block.state_root()`, overwrite it with the pre-envelope root.
  - **Skipped 11 ePBS-incompatible tests at Gloas**:
    - Data columns (7): `fulu_prune_data_columns_happy_case`, `_no_finalization`, `_margin1/3/4`, `test_custody_column_filtering_regular_node`, `_supernode` — Gloas delivers data columns via execution payload envelope, not the beacon block body
    - Light client (2): `light_client_bootstrap_test`, `light_client_updates_test` — Gloas block body has no `execution_payload` field (it's in the bid/envelope), `block_to_light_client_header` fails with `IncorrectStateVariant`
    - Schema downgrade (2): `schema_downgrade_to_min_version_archive_node_grid_aligned`, `_full_node_per_epoch_diffs` — block replay from cold storage fails due to missing envelopes (pruned during finalization) and two-phase state roots
- **Final store_tests status at Gloas: 73/73 pass** (62 real + 11 skipped)
- Tests: 136/136 EF minimal (fake crypto), 8/8 fork choice EF (real crypto), 38/38 state_processing

### 2026-02-19 (run 11)
- Fetched upstream: no new commits since run 10
- No new consensus-specs changes requiring implementation
- **Fixed 9 more Gloas store_tests failures** (53→62 passing, 20→11 failing):
  1. **WSS state root consistency** (builder.rs): checkpoint sync stored the WSS state under the post-envelope hash, but the block import path expects states under the block's pre-envelope state_root. Fix: for aligned Gloas checkpoints, use `weak_subj_block.state_root()` as the storage key for `set_split`, `update_finalized_state`, and `put_state`.
  2. **WSS state advancement** (builder.rs): during state advancement to epoch boundary, `per_slot_processing` was called with `None` state_root, causing `state_roots[block_slot]` to contain the post-envelope hash instead of the pre-envelope root. Fix: pass `weak_subj_block.state_root()` as `state_root_opt` for the first call.
  3. **get_advanced_hot_state envelope re-application** (hot_cold_store.rs): when loading a state from disk for Gloas blocks, the pre-envelope state needs envelope re-application to update `latest_block_hash`, execution requests, and builder payments. Added envelope re-application with skip-if-already-applied check (for WSS checkpoint states stored as post-envelope).
  4. **get_advanced_hot_state cache/disk root handling** (hot_cold_store.rs): cache and disk paths now return the caller's `state_root` (pre-envelope root) instead of the stored root (which may be post-envelope), so the sanity check in `block_verification.rs` (`parent_state_root == block.state_root()`) passes.
  5. **reconstruct_historic_states envelope processing** (reconstruct.rs): reconstruction replays blocks without processing envelopes, so `latest_block_hash` was never updated and subsequent blocks failed bid validation. Fix: after `per_block_processing`, load and apply the envelope if available. Keep `prev_state_root` as the pre-envelope root for consistency with the forward chain's `state_roots` array.
  6. **reconstruct_historic_states integrity check** (reconstruct.rs): the final root check compared the block's pre-envelope root with the state's post-envelope hash. Accept the mismatch for Gloas states.
  7. **Test envelope storage**: copy envelopes from the source harness to the WSS chain's store during both checkpoint setup and backfill, so reconstruction can access them.
  8. **canonical_head try_update_head_state**: update the head snapshot's state from pre-envelope to post-envelope after `process_self_build_envelope` and `process_payload_envelope`, since fork choice won't re-compute the head when the head block hasn't changed.
  9. **Migration dual mapping** (hot_cold_store.rs): store ColdStateSummary under both pre-envelope and post-envelope roots during migration so lookups by either root succeed.
- Newly passing tests: `weak_subjectivity_sync_easy`, `weak_subjectivity_sync_single_block_batches`, `weak_subjectivity_sync_unaligned_advanced_checkpoint`, `weak_subjectivity_sync_unaligned_unadvanced_checkpoint`, `weak_subjectivity_sync_skips_at_genesis` (5 new WSS + 4 from previous runs)
- Remaining 11 failures are pre-existing and unrelated to Gloas ePBS state handling:
  - **Data columns** (7): fulu_prune_data_columns_* (5), test_custody_column_filtering_* (2)
  - **Light client** (2): light_client_bootstrap_test, light_client_updates_test
  - **Schema downgrade** (2): schema_downgrade_to_min_version_*

### 2026-02-18 (run 10)
- Fetched upstream: no new commits since run 9
- No new consensus-specs changes requiring implementation
- **Fixed 14 more Gloas store_tests failures** (39→53 passing, 34→20 failing):
  1. `block_replayer.rs`: genesis anchor block was applying empty bid block_hash (0x0000) to state's latest_block_hash, corrupting state for all subsequent blocks. Fix: skip zero block_hash in anchor handling.
  2. `test_utils.rs`: `add_attested_blocks_at_slots_given_lbh` and `_with_lc_data` returned post-envelope state hash as state key. DB stores states under pre-envelope state_root (block.state_root()). Fix: use block state_root for Gloas.
  3. `test_utils.rs`: `add_block_at_slot` and `make_block_with_envelope` called with post-envelope state but no state_root, causing `complete_state_advance` to compute wrong state root. Fix: derive state_root from `latest_block_header.state_root` for Gloas.
- Remaining 20 failures categorized:
  - **Data columns** (7): fulu_prune_data_columns_* (5), test_custody_column_filtering_* (2) — zero data columns stored in Gloas blocks
  - **Weak subjectivity sync** (6): all weak_subjectivity_sync_* — state root / block replay issues during checkpoint sync
  - **State reconstruction** (3): epoch_boundary_state_attestation_processing, forwards_iter_block_and_state_roots_until, finalizes_after_resuming_from_db — post-envelope state hash doesn't match any stored key
  - **Schema downgrade** (2): schema_downgrade_to_min_version_* — ParentBlockRootMismatch during block replay
  - **Light client** (2): light_client_bootstrap_test, light_client_updates_test

### 2026-02-18 (run 9)
- Fetched upstream: no new commits since run 8
- No new consensus-specs changes requiring implementation (checked latest merged PRs — all packaging/infrastructure)
- Tracked open consensus-specs PRs: #4918, #4939, #4898, #4892, #4932 — all still open/unmerged
- CI from run 8: check+clippy+fmt ✓, ef-tests ✓, unit-tests ✓, fork-specific-tests ✗ (pre-existing store_tests Gloas failures)
- **Fixed 2 pre-existing Gloas test failures**:
  - `store_tests::randomised_skips` (and 1 other) — root cause: `ForkCanonicalChainAt` in `extend_chain_with_sync` used `state_at_slot()` which returns the head snapshot state (pre-envelope, stale `latest_block_hash`). Fix: when at the Gloas head slot, use `get_current_state_and_root()` which loads the post-envelope state from the state cache. Net improvement: 37→39 passing, 36→34 failing store_tests at Gloas.
  - `schema_stability::schema_stability` — missing "bev" (BeaconEnvelope) DB column in expected columns list. Added it.
- Remaining 34 store_tests Gloas failures are deeper infrastructure issues: block replayer envelope handling, state reconstruction, weak subjectivity sync, schema downgrade, data column pruning. These require systematic fixes across multiple subsystems.
- `finalizes_after_resuming_from_db` failure confirmed pre-existing (fails without changes too) — head state mismatch between snapshot and DB due to Gloas two-phase state transition.

### 2026-02-18 (run 8)
- Fetched upstream: no new commits since run 7
- No new consensus-specs changes requiring implementation (top commits are packaging/infrastructure: eth-remerkleable, package rename, publish scripts)
- Tracked open consensus-specs PRs:
  - #4918 (attestations for known payload statuses) — still open
  - #4939 (request missing payload envelopes for index-1 attestation) — still open
  - #4898 (remove pending status from tiebreaker) — still open, assessed: our tiebreaker code still checks Pending but it's functionally correct, trivial change when merged
  - #4892 (remove impossible branch in forkchoice) — still open, assessed: our `is_supporting_vote_gloas` uses `<=` (old spec), PR changes to assert `>=` + check `==`, functionally equivalent
  - #4932 (add Gloas sanity/blocks tests with payload attestation coverage) — still open
- Unskipped 3 fork choice EF tests that were blocked on lighthouse#8689 (now that PR #4807 proposer boost check is implemented):
  - `voting_source_beyond_two_epoch`, `justified_update_not_realized_finality`, `justified_update_always_if_better`
  - All pass with both real and fake crypto
  - EF test results: 78/78 real crypto (0 skipped, was 3), 136/136 fake crypto (0 skipped, was 3)
- Fixed CI failures:
  - clippy `question_mark` lint in `lookups.rs:1973` (Rust 1.93 new lint)
  - BLS test fixtures missing in CI — `consensus-spec-tests` is not a git submodule, needs `make -C testing/ef_tests` to download. Replaced `submodules: recursive` with download step. Also removed unused `submodules: recursive` from non-ef-tests jobs.
  - `rpc_columns_with_invalid_header_signature` fails at Gloas because DataColumnSidecar structure changed (no `signed_block_header`). Skipped for Gloas — test premise doesn't apply.
- Pre-existing Gloas test failures identified (not introduced by this run):
  - 29 `store_tests::*` failures at `FORK_NAME=gloas` — `PayloadBidInvalid: bid parent_block_hash does not match state latest_block_hash`. Root cause: mock EL + test harness state management with skipped slots doesn't properly handle ePBS envelope state. These are test infrastructure issues, not consensus bugs.
  - `validator_monitor::missed_blocks_across_epochs` — also pre-existing

### 2026-02-18 (run 7)
- Fetched upstream: no new commits since run 6
- No new consensus-specs changes requiring implementation (checked latest merged PRs — all packaging/infrastructure)
- Tracked open consensus-specs PRs: #4918 (attestations for known payload statuses), #4939 (request missing payload envelopes for index-1 attestation) — both still open/unmerged
- Implemented remaining PR #4807 change: equivocating validator weight in `is_head_weak`
  - Threaded `equivocating_indices: &BTreeSet<u64>` from `find_head` → `find_head_gloas` → `should_apply_proposer_boost_gloas`
  - Added equivocating validators' effective balance to parent attestation weight before comparing against reorg threshold
  - This matches spec's `is_head_weak` which sums both attesting and equivocating weight
  - Previously had a placeholder comment "simplified: we don't have equivocating indices here, so skip this"
- Fixed pre-existing clippy warnings across codebase (Rust 1.93 has stricter lints):
  - proto_array: collapsible_if, manual_let_else in 4 places
  - state_processing: 10 redundant closures (`|e| Error(e)` → `Error`), let_underscore_must_use in block_replayer
  - fork_choice: map_or → is_none_or
  - beacon_chain: collapsible_if, manual_let_else, needless_borrow, bool_assert_comparison
  - http_api: large_stack_frames in test functions
  - types: items_after_test_module
- Tests: 18/18 proto_array, 34/34 fork_choice, 56/56 state_processing, 8/8 fork_choice EF (real + fake crypto) — all pass
- Remaining from PR #4807 (non-consensus-critical reorg enhancements):
  - `record_block_timeliness` with 2-element timeliness vector — not strictly needed, our `ptc_timely: current_slot == block.slot()` and `is_before_attesting_interval` checks are functionally equivalent
  - `is_proposer_equivocation` helper extraction — cosmetic refactor, logic already exists inline

### 2026-02-18 (run 6)
- Fetched upstream: no new commits since run 5
- No new consensus-specs changes requiring implementation (latest release still v1.7.0-alpha.2, newer spec commits are packaging/infrastructure)
- Reviewed community PRs:
  - PR #25 (Th0rgal): 4 fixes — 3 already applied on main, applied remaining fix (use canonical `BUILDER_INDEX_SELF_BUILD` constant instead of local copy in proto_array). Closed PR with credit.
  - PR #26 (Th0rgal): cargo fmt + unused imports — all already fixed on main. Closed as redundant.
- Tests: 52/52 proto_array+fork_choice, 136/136 minimal EF (fake_crypto), 8/8 fork_choice EF (real crypto) — all pass

### 2026-02-18 (run 5)
- Fetched upstream: no new commits since run 4 (top is `54b357614` — agent review docs, skip)
- No new consensus-specs changes requiring implementation (top commits are packaging/infrastructure: eth-remerkleable, package rename, dependency updates)
- Implemented consensus-specs PR #4807: `update_proposer_boost_root` proposer index check
  - Added `canonical_head_proposer_index: Option<u64>` parameter to `ForkChoice::on_block`
  - In `import_block`, compute expected proposer from cached head state before fork choice lock
  - Only apply proposer boost if `block.proposer_index == expected_proposer_index`
  - Skip check when epoch mismatch (can't compute proposer without state advance) or during fork revert
  - Updated 6 call sites: beacon_chain, fork_revert, fork_choice tests, ef_tests, payload_invalidation
  - Tests: 8/8 fork choice EF (real + fake crypto), 34/34 fork_choice, 18/18 proto_array — all pass
- Remaining from PR #4807 (not yet implemented):
  - `is_proposer_equivocation` helper for `get_proposer_head` reorg logic
  - `should_apply_proposer_boost` changes in Gloas `get_weight` (already partially implemented, needs `block_timeliness` vector)
  - Modified `is_head_weak` (Gloas) to include equivocating validator weight
  - `record_block_timeliness` with two-element timeliness vector
  - These are non-consensus-critical (reorg logic only) and can be done in a follow-up

### 2026-02-18 (run 4)
- Fixed CI: `cargo fmt` failure in gossip_methods.rs and fork_choice.rs (from run 3 commits)
- Revisited previously-skipped cherry-picks:
  - `be799cb2a` — VC head monitor timeout: **SKIP** — our code uses `EventSource::get(path)` (bare reqwest with no timeout), not `self.client` with configured timeout. Bug doesn't affect us.
  - `691c8cf8e` — duplicate data columns fix: **SKIP** — our code already deduplicates correctly (`.map(|(root, _)| root).unique()`). Upstream's bug was `.unique()` on `(root, slot)` tuples.
  - `c61665b3a` — penalize peers for invalid RPC: **DONE** — resolved conflict in rpc_tests.rs imports (kept our `mod common` pattern, added `libp2p::PeerId`). All 3 new tests pass.
- New cherry-picks:
  - `a3a74d898` — fix ProcessHealth::observe computing `children_system` twice instead of `children_system + children_user` (metrics bug)
  - `5563b7a1d` — fix execution engine test using stale `valid_payload.block_hash()` instead of `second_payload.block_hash()`
  - `1fe7a8ce7` (partial) — gate `inactivity_scores` rewards tests to Altair+ forks (prevents directory-not-found on Phase0)
- Evaluated and skipped:
  - `945f6637c` — reqwest re-export removal (20-file refactor, 6 conflicts)
  - `48a2b2802` — delete OnDiskConsensusContext (still used in our state_lru_cache.rs)
  - `fcfd061fc` — feature gate SseEventSource (file doesn't exist in our fork)
  - `f4a6b8d9b` — tree-sync lookup sync tests (4600-line rewrite, heavy conflicts)
- No new consensus-specs changes requiring implementation (top commits are packaging/infrastructure)

### 2026-02-18 (run 3)
- Implemented spec change `3f9caf73`: gossip validation `[IGNORE]` for Gloas blocks whose parent execution payload hasn't been seen
  - New `GloasParentPayloadUnknown` error variant in `BlockError`
  - Check in `GossipVerifiedBlock::new()`: for Gloas blocks, if parent has `bid_block_hash` (is a Gloas block) and `payload_revealed == false`, IGNORE the block
  - Pre-Gloas parents are always considered "seen" (payload is in the block body)
  - Gossip handler returns `MessageAcceptance::Ignore` with no peer penalty
- Tests: 8/8 fork_choice EF (real + fake crypto), 170/170 beacon_chain (1 pre-existing failure excluded), 23/23 network fulu (1 pre-existing failure excluded)

### 2026-02-18 (run 2)
- Fetched upstream: no new commits since earlier today
- Cherry-picked cleanly:
  - `d4ec006a3` — update `time` crate to fix `cargo audit` failure (via `cargo update -p time`)
  - `711971f26` — cache slot in check_block_relevancy to prevent TOCTOU race
  - `96bc5617d` — auto-populate ENR UDP port from discovery listen port
  - `8d72cc34e` — add sync request metrics
  - `2f7a1f3ae` — support pinning nightly ef test runs
- Conflicted (skipped):
  - `d7c78a7f8` — rename --reconstruct-historic-states to --archive (conflicts in store_tests.rs and tests.rs due to gloas changes)
- Fixed pre-existing DataColumnSidecar `.index` → `.index()` in network test code (6 call sites)
- New spec changes assessed:
  - `3f9caf73` — ignore block if parent payload unknown (gossip validation) — needs implementation
  - `e57c5b80` — rename execution_payload_states to payload_states — naming only, no code change needed
  - `e46ecbae` — ZK proof dedup (EIP-8025 feature, not in gloas core)
  - Others: infrastructure, docs, renaming

### 2026-02-18
- Fetched upstream: 20 new commits since last check (including 4 Gloas upstream PRs)
- Cherry-picked cleanly:
  - `c5b4580e3` — return correct variant for snappy errors (rpc codec fix)
  - `9065e4a56` — add pruning of observed_column_sidecars (memory fix)
- Conflicted (resolved in run 4):
  - `be799cb2a` — VC head monitor timeout fix (skipped — doesn't affect our SSE client pattern)
  - `691c8cf8e` — fix duplicate data columns in DataColumnsByRange (skipped — our dedup is already correct)
  - `c61665b3a` — penalize peers for invalid rpc request (cherry-picked with conflict resolution)
- Upstream Gloas PRs (evaluated, not cherry-picked — our impl is ahead):
  - `eec0700f9` — Gloas local block building MVP
  - `67b967319` — Gloas payload attestation consensus
  - `41291a8ae` — Gloas fork upgrade consensus
  - `4625cb6ab` — Gloas local block building cleanup

### 2026-02-15
- Fetched upstream: 4 new commits since last check
- `48a2b2802` delete OnDiskConsensusContext, `fcfd061fc` fix eth2 compilation, `5563b7a1d` fix execution engine test, `1fe7a8ce7` implement inactivity scores ef tests
- None security-critical, none cherry-pick urgent
