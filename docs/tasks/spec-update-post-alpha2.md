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

## Upcoming Spec Test PRs (not yet merged)

- **PR #4940** — "Add initial fork choice tests for Gloas": tests `on_execution_payload` (EMPTY→FULL transition), basic head tracking. Our `ForkChoiceHandler` already supports `on_execution_payload` steps and `head_payload_status` checks — ready to pass when merged.
- **PR #4932** — "Add Gloas sanity/blocks tests with payload attestation coverage": tests `process_payload_attestation` during full block processing. Our `SanityBlocksHandler` runs all forks — ready to pass when merged.
- **PR #4960** — "Add Gloas fork choice test for new validator deposit": extends fork choice tests with deposit scenarios. Already supported by our handler.

## Progress log

### 2026-03-08 — spec conformance deep audit (run 555)
- Verified `process_execution_payload_bid` against latest spec: all 9 validation checks in exact order, state mutations correct
- Verified `process_execution_payload_envelope` against latest spec: all 10 checks + state mutations match
- Verified helpers: `is_active_builder`, `can_builder_cover_bid`, `get_pending_balance_to_withdraw_for_builder` all correct
- Verified `is_supporting_vote_gloas_at_slot` already handles the "impossible branch" from PR #4892 correctly
- EIP-7843 (SLOTNUM opcode, PR #4840): adds `slot_number` field to `PayloadAttributes` — NOT YET MERGED, will need implementation when it lands
- PR #4939 (request missing envelopes for index-1 attestation): P2P guidance, still open
- No discrepancies found between vibehouse implementation and spec

### 2026-03-08 — reviewed upcoming spec test PRs
- Checked open Gloas-related spec PRs: #4940 (fork choice), #4932 (sanity/blocks), #4960 (fork choice + deposit)
- All test formats already supported by our EF test handlers (ForkChoiceHandler, SanityBlocksHandler)
- PTC Lookbehind (PR #4979) still open, no new changes since last check
- No Gloas-related spec changes merged since last audit (all recent merges are dep updates/tooling)

### 2026-03-08 — second scan: two new merged PRs, one upcoming
- PR #4950 (by_root serve range extension): no code change needed, our handler is compliant
- PR #4947 (pre-fork proposer_preferences subscription): already implemented
- PR #4979 (PTC Lookbehind): open, significant spec change, tracked above

### 2026-03-09 — spec scan + internal crate rebrand (run 616)
- All 9 tracked PRs still OPEN: #4979, #4940, #4932, #4960, #4962, #4840, #4843, #4630, #4939
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related spec PRs since last scan
- Rebranded `lighthouse_validator_store` crate → `vibehouse_validator_store`, `LighthouseValidatorStore` struct → `VibehouseValidatorStore` (21 files, directory rename)
- All tests pass, clippy clean

### 2026-03-09 — spec scan + help text rebrand (run 614)
- All 6 tracked PRs still OPEN: #4979, #4940, #4932, #4960, #4840, #4939
- No new consensus-specs release (still v1.7.0-alpha.2)
- Rebranded remaining help text: telemetry service name defaults (lighthouse-bn/vc → vibehouse-bn/vc), lcli description
- Regenerated CLI help docs (7 help_*.md files updated)
- Verified: all /lighthouse/ API endpoints intentionally kept (established API path, external tooling depends on it)
- Verified: Docker lighthouse symlink + kurtosis cl_type=lighthouse intentionally kept (infrastructure constraints)

### 2026-03-09 — spec scan + deep rebrand (run 612)
- All 9 tracked PRs still open, no new merges, no new release
- Completed deep rebrand: CLI help text, error/log messages, default data directory (~/.lighthouse → ~/.vibehouse), metrics name (lighthouse_info → vibehouse_info), boot_node about text, validator_manager/account_manager help strings — 18 files, 51 line changes
- 359 tests pass, full lint clean

### 2026-03-09 — spec scan + identity rebranding (run 611)
- Spec scan in progress (background agent checking all 9 tracked PRs)
- Rebranded user-visible identity strings from Lighthouse to vibehouse (version, CLI, monitoring, P2P agent, telemetry)
- All tests passing, full lint clean

### 2026-03-09 — spec scan (run 610)
- All 9 tracked PRs still open, no status changes: #4979 (PTC lookbehind, design still evolving — potuz favors 2*SLOTS_PER_EPOCH, ensi321/nflaig exploring smaller options), #4940, #4932, #4960, #4962, #4840, #4843, #4630, #4939
- No new Gloas spec PRs merged since run 609
- No new consensus-specs release (still v1.7.0-alpha.2)
- CI: latest push (run 609 cleanup) in progress, all nightly tests passing through Mar 8
- Codebase audit: zero clippy warnings, zero compiler warnings, zero TODOs in Gloas code, comprehensive metrics coverage for ePBS gossip types
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan + cleanup (run 609)
- All 9 tracked PRs still open: #4979 (PTC lookbehind, active discussion Mar 7 re: Vector size trade-offs), #4940, #4932, #4960, #4962, #4840, #4839, #4843, #4630
- PR #4950 (by_root serve range) merged Mar 6 — already compliant (tracked above)
- No new Gloas spec changes merged since run 608
- PTC Lookbehind (#4979) discussion: potuz/jtraglia/ensi321/nflaig debating Vector size (2*SLOTS_PER_EPOCH vs 2 vs 1), asserts for `get_ptc`, next epoch JIT computation. Design still evolving.
- Codebase cleanup: consolidated 10 duplicate TODO comments in beacon_chain builder
- Full code quality scan: no unwraps in production Gloas code, no dead_code allows, no stale warp references, clippy clean, no compiler warnings

### 2026-03-09 — spec scan (run 608)
- All 6 tracked PRs still open: #4979 (PTC lookbehind), #4940, #4932, #4960, #4840, #4939
- PR #4948 (reorder PayloadStatus constants: EMPTY=0, FULL=1, PENDING=2) merged Feb 26 — already matches our `GloasPayloadStatus` enum ordering
- Cleaned up stale warp references in doc comments + removed dead `config` field from `AppState` (post warp→axum migration)
- No new Gloas spec changes requiring code updates

### 2026-03-08 — audit found all changes already implemented
- Compared consensus-specs master against v1.7.0-alpha.2 tag
- 4 Gloas spec files changed: beacon-chain.md, fork-choice.md, p2p-interface.md, validator.md
- All consensus-critical changes (PayloadAttestationData, PayloadStatus, dual PTC votes, is_pending_validator, should_extend_payload, validate_on_attestation) were already in vibehouse
- vibehouse was implementing from spec PRs ahead of the release tag
- validator.md changes are documentation-only (section renaming)
- Config changes: removals of deprecated fields, Heze renaming — not relevant to vibehouse
