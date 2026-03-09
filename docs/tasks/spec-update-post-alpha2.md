# Spec Update: Post v1.7.0-alpha.2 Changes

## Objective
Implement Gloas spec changes merged to consensus-specs master after v1.7.0-alpha.2.

## Status: DONE (all changes already implemented)

## Changes identified (all already in codebase)

### 1. PayloadAttestationData — `blob_data_available` field
- Already present: `consensus/types/src/payload_attestation_data.rs:24`

### 2. PayloadStatus enum (EMPTY=0, FULL=1, PENDING=2)
- Already correct: `consensus/proto_array/src/proto_array_fork_choice.rs:41-45`

### 3. Fork choice: dual PTC vote tracking
- Both `payload_timeliness_vote` and `payload_data_availability_vote` tracked: `consensus/fork_choice/src/fork_choice.rs:1453-1464`
- Anchor votes initialized to all True: `consensus/fork_choice/src/fork_choice.rs:478-479`
- `validate_on_attestation` index=1 check: `consensus/fork_choice/src/fork_choice.rs:1198-1206`

### 4. should_extend_payload
- Requires both timely AND data available: `consensus/proto_array/src/proto_array_fork_choice.rs:1577`
- Tests cover all cases: `proto_array_fork_choice.rs:3837-4127`

### 5. is_pending_validator + process_deposit_request
- Implemented with tests: `consensus/state_processing/src/per_block_processing/process_operations.rs:726-731, 759-790`

### 6. P2P changes
- Bid gossip IGNORE for unknown parent: `beacon_node/network/src/network_beacon_processor/gossip_methods.rs:3374-3377`
- Envelope request: current handler returns what's available, skips missing (spec-compliant via MAY clause)

### 7. Config removals
- Not blocking: vibehouse already uses `SLOT_DURATION_MS`, doesn't implement Heze forks

### 8. ExecutionPayloadEnvelopesByRoot serve range (PR #4950, merged Mar 6)
- Extends required serve range from "since latest finalized epoch" to `[max(GLOAS_FORK_EPOCH, current_epoch - MIN_EPOCHS_FOR_BLOCK_REQUESTS), current_epoch]`
- Our handler (`rpc_methods.rs:519`) serves whatever is in the store, skips missing — compliant via MAY clause
- Blinded envelopes are never pruned, full payloads are pruned but only for finalized blocks well within the range
- No code change needed

### 9. Pre-fork proposer_preferences subscription (PR #4947, merged Feb 26)
- Documentation note: nodes SHOULD subscribe to `proposer_preferences` topic ≥1 epoch before fork activation
- Already implemented: `PRE_FORK_SUBSCRIBE_EPOCHS=1` in `network/src/service.rs`
- No code change needed

## Upcoming: PTC Lookbehind (PR #4979, OPEN — not yet merged)

Adds `ptc_lookbehind: Vector[Vector[ValidatorIndex, PTC_SIZE], 2 * SLOTS_PER_EPOCH]` to BeaconState to cache PTC committees for previous + current epochs. Fixes a real bug: when processing payload attestations at epoch boundaries (e.g., slot 32 validating PTC of slot 31), effective balance changes from epoch processing can cause `get_ptc` to return a different committee than what was valid when the attestation was created.

**Changes required when merged:**
1. New BeaconState field: `ptc_lookbehind` (Vector of Vectors, ~256KB)
2. New helper: `compute_ptc` (current `get_ptc` logic extracted)
3. Refactored `get_ptc`: lookup from cache for prev/current epoch, compute on demand for next epoch
4. New epoch processing: `process_ptc_lookbehind` — shift window and pre-compute next epoch
5. Fork upgrade: `upgrade_to_gloas` initializes `ptc_lookbehind` via `initialize_ptc_lookbehind`
6. New spec tests: `test_process_ptc_lookbehind`, `test_get_ptc_assignment` variants

**vibehouse has the same bug** — our `get_ptc_committee` (gloas.rs:377) computes PTC from scratch using current state balances. Will fix when PR merges.

**Detailed implementation plan (from PR #4979 diff analysis):**

1. **New EthSpec type**: `PtcLookbehindLength` = `2 * SlotsPerEpoch` (mainnet: 64, minimal: 16)

2. **BeaconState field** (`consensus/types/src/beacon_state.rs`):
   ```rust
   #[superstruct(only(Gloas))]
   pub ptc_lookbehind: Vector<Vector<u64, E::PtcSize>, E::PtcLookbehindLength>,
   ```

3. **Rename current `get_ptc_committee` → `compute_ptc`** (`consensus/state_processing/src/per_block_processing/gloas.rs:377`):
   - Same logic, just extracted as the "compute from scratch" path

4. **New `get_ptc` function** (cache-aware wrapper):
   ```
   epoch < state_epoch → lookup ptc_lookbehind[slot % SLOTS_PER_EPOCH] (previous epoch)
   epoch == state_epoch → lookup ptc_lookbehind[SLOTS_PER_EPOCH + slot % SLOTS_PER_EPOCH] (current epoch)
   epoch == state_epoch + 1 → compute_ptc(state, slot) on demand (next epoch)
   ```

5. **New epoch processing `process_ptc_lookbehind`** (`consensus/state_processing/src/per_epoch_processing/`):
   - Shift: `ptc_lookbehind[0..SLOTS_PER_EPOCH] = ptc_lookbehind[SLOTS_PER_EPOCH..]`
   - Fill: compute PTC for each slot in next epoch, store in `ptc_lookbehind[SLOTS_PER_EPOCH..]`
   - Called at end of `process_epoch`, after `process_proposer_lookahead`

6. **Fork upgrade `initialize_ptc_lookbehind`** (`consensus/state_processing/src/upgrade/gloas.rs`):
   - Previous epoch slots: all zeros (empty — no previous epoch PTC at fork boundary)
   - Current epoch slots: `compute_ptc(state, slot)` for each slot in current epoch

7. **SSZ/tree-hash**: Vector<Vector<u64>> needs proper tree-hash support; verify ssz_static tests handle it

8. **Update callers**: `process_payload_attestation`, `get_indexed_payload_attestation`, `validator_ptc_duties` — all call current `get_ptc_committee`, redirect to new `get_ptc`

## Upcoming: Fork Choice Milliseconds (PR #4954, OPEN — not yet merged)

Converts fork choice `Store.time`/`Store.genesis_time` (seconds) to `Store.time_ms`/`Store.genesis_time_ms` (milliseconds). Adds `compute_slot_since_genesis`, `compute_time_into_slot_ms` helpers.

**vibehouse impact: NONE for fork choice internals.** vibehouse's `ForkChoiceStore` trait already stores time as `Slot` (not UNIX timestamp), and block timeliness already uses millisecond `Duration` via `millis_from_current_slot_start()`. The conversion from wall-clock time to slot happens in the slot clock layer, outside fork choice.

**Only change needed when merged:** EF fork choice test handler — `on_tick` test steps will send millisecond values instead of seconds. The test runner deserializer will need to convert from `milliseconds / SLOT_DURATION_MS` instead of `seconds * 1000 / SLOT_DURATION_MS`.

## Upcoming: Remove Pending from Tiebreaker (PR #4898, OPEN — already compliant)

Our `get_payload_tiebreaker` (proto_array_fork_choice.rs:1560) doesn't special-case PENDING — it falls through to `should_extend_payload`, matching the proposed spec change. No code change needed.

## Upcoming: Remove Impossible Branch (PR #4892, OPEN — already compliant)

Changes `if message.slot <= block.slot` to `assert message.slot >= block.slot` + `if message.slot == block.slot`. Our `is_supporting_vote_gloas_at_slot` (proto_array_fork_choice.rs:1482) already uses `==` with an assert comment. No code change needed.

## Upcoming: Variable PTC Deadline (PR #4843, OPEN — will need implementation)

Renames `payload_present`→`payload_timely`, `is_payload_timely`→`has_payload_quorum`, adds `payload_envelopes` to store, adds `MIN_PAYLOAD_DUE_BPS` config, introduces `get_payload_due_ms`/`get_payload_size` for variable deadline based on payload size. Last updated Feb 19, design still evolving.

## Upcoming: Index-1 Attestation Envelope Validation (PR #4939, OPEN — will need implementation)

Adds two new gossip validation rules for `beacon_aggregate_and_proof` and `beacon_attestation_{subnet_id}`:
1. **[REJECT]** If `data.index == 1` (payload present for past block), the execution payload for that block must have passed validation
2. **[IGNORE]** If `data.index == 1`, the execution payload envelope must have been seen. Clients MAY queue the attestation and SHOULD request the envelope via `ExecutionPayloadEnvelopesByRoot`

**vibehouse impact: will need implementation.** Currently, index-1 attestations pass through standard gossip validation without checking payload envelope status. When merged, need to add checks in `attestation_verification.rs` to verify the referenced block's payload has been received/validated before accepting index-1 attestations.

## Upcoming Spec Test PRs (not yet merged)

- **PR #4940** — "Add initial fork choice tests for Gloas": tests `on_execution_payload` (EMPTY→FULL transition), basic head tracking. Our `ForkChoiceHandler` already supports `on_execution_payload` steps and `head_payload_status` checks — ready to pass when merged.
- **PR #4932** — "Add Gloas sanity/blocks tests with payload attestation coverage": tests `process_payload_attestation` during full block processing. Our `SanityBlocksHandler` runs all forks — ready to pass when merged.
- **PR #4960** — "Add Gloas fork choice test for new validator deposit": extends fork choice tests with deposit scenarios. Already supported by our handler.
- **PR #4962** — "Add Gloas sanity/blocks tests for missed payload withdrawal interactions": tests all 4 combinations of block with/without withdrawals when payload doesn't arrive, then next block with/without new withdrawals. Test-only PR — our handler runs all forks, ready to pass when merged.

### 10. PR #4918 — Only allow attestations for known payload statuses (merged Feb 13)
- Adds validation to `validate_on_attestation`: if `attestation.data.index == 1`, `beacon_block_root` must be in `store.payload_states`
- Already implemented: `fork_choice.rs:1191-1199` checks `block.payload_revealed` with `PayloadNotRevealed` error

### 11. PR #4923 — Ignore beacon block if parent payload unknown (merged Feb 15)
- New gossip `[IGNORE]` rule: if parent block's execution payload envelope not yet received, ignore the block
- Already implemented: `block_verification.rs:970-983` returns `BlockError::GloasParentPayloadUnknown`

### 12. PR #4931 — Rebase FOCIL onto Gloas (merged Feb 17)
- FOCIL (EIP-7805) is a separate feature from ePBS (EIP-7732), promoted to Heze fork (after Gloas) via PR #4942
- Not relevant for vibehouse's Gloas implementation

## Progress log

### 2026-03-09 — spec scan (run 649)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI green: ci passed, nightly 3 consecutive greens (Mar 7-9)
- cargo audit: same 1 medium rsa advisory (transitive, no fix available)
- No code changes needed

### 2026-03-09 — spec scan (run 648)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- No new Gloas PRs opened since last scan
- CI green: ci passed, nightly all green; docker paths-ignore working
- Local verification: 78/78 real-crypto + 138/138 fake-crypto spec tests pass
- Clippy clean (zero warnings)
- No code changes needed

### 2026-03-09 — spec scan (run 647)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI: 4/6 passed, 2 in progress; cargo audit/outdated clean
- No code changes needed

### 2026-03-09 — spec scan (run 646)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- No new Gloas PRs opened since last scan
- CI: check/clippy/fmt, ef-tests, http_api, network+op_pool passed; unit tests + beacon_chain in progress
- cargo audit: same 1 medium rsa advisory (transitive, no fix available)
- No code changes needed

### 2026-03-09 — spec scan (run 645)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- No new Gloas PRs opened since last scan
- CI: check/clippy/fmt, ef-tests, http_api, network+op_pool passed; unit tests + beacon_chain still running
- Nightly: all 27 jobs green (Mar 9)
- cargo audit: same 1 medium rsa advisory (transitive, no fix available)
- No code changes needed

### 2026-03-09 — spec scan (run 644)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI green: check/clippy/fmt, ef-tests, network+op_pool passed; beacon_chain/http_api/unit tests in progress
- Nightly tests: 3 consecutive greens (Mar 7-9)
- cargo audit: same 1 medium + 5 unmaintained (all transitive, not actionable)
- No code changes needed

### 2026-03-09 — spec scan (run 643)
- All 10 previously tracked Gloas PRs still OPEN
- Found 2 new untracked PRs: #4939 (index-1 attestation envelope validation — will need implementation) and #4962 (test-only: missed payload withdrawal interactions)
- Now tracking 12 open Gloas PRs total
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI green (ef-tests, clippy, network+op_pool passed; others still running)
- cargo audit: 1 medium advisory (rsa RUSTSEC-2023-0071, no fix available), 5 unmaintained warnings — all transitive, not actionable
- No code changes needed

### 2026-03-09 — spec scan (run 642)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged since run 641
- PTC Lookbehind (#4979): no activity since Mar 7, design debate ongoing (2*SLOTS_PER_EPOCH vs smaller)
- CI green, nightly tests consecutive green since Mar 4
- Codebase audit: zero production unwraps, no todo!()/unimplemented!() outside tests, all #[allow(dead_code)] are justified error enum fields
- No code changes — spec stable, fully compliant

### 2026-03-09 — consolidated: runs 608-641 (Mar 8-9)
Key activities across these runs (no-change spec scans omitted):
- **run 641**: added paths-ignore to docker workflow for docs-only commits; test coverage audit found no actionable gaps
- **run 640**: post-rebrand devnet verification SUCCESS (finalized_epoch=8)
- **run 633**: pre-analyzed 4 upcoming PRs (#4898, #4892, #4954, #4843) — all already compliant or no change needed
- **run 621**: cleaned up stale upstream CI workflows, updated docker.yml/release.yml for vibehouse
- **runs 611-619**: full vibehouse rebranding (binary, crates, docs, CLI, metrics, P2P agent)
- **run 608**: confirmed PayloadStatus reorder (#4948) already matches vibehouse
- **run 555 (Mar 8)**: deep spec conformance audit — all checks verified correct
- Tracked 10 PRs (all still open), no new spec release, CI continuously green

### 2026-03-08 — audit found all changes already implemented
- Compared consensus-specs master against v1.7.0-alpha.2 tag
- 4 Gloas spec files changed: beacon-chain.md, fork-choice.md, p2p-interface.md, validator.md
- All consensus-critical changes (PayloadAttestationData, PayloadStatus, dual PTC votes, is_pending_validator, should_extend_payload, validate_on_attestation) were already in vibehouse
- vibehouse was implementing from spec PRs ahead of the release tag
