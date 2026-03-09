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

## Upcoming Spec Test PRs (not yet merged)

- **PR #4940** — "Add initial fork choice tests for Gloas": tests `on_execution_payload` (EMPTY→FULL transition), basic head tracking. Our `ForkChoiceHandler` already supports `on_execution_payload` steps and `head_payload_status` checks — ready to pass when merged.
- **PR #4932** — "Add Gloas sanity/blocks tests with payload attestation coverage": tests `process_payload_attestation` during full block processing. Our `SanityBlocksHandler` runs all forks — ready to pass when merged.
- **PR #4960** — "Add Gloas fork choice test for new validator deposit": extends fork choice tests with deposit scenarios. Already supported by our handler.

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

### 2026-03-09 — spec scan + post-rebrand devnet verification (run 640)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged or opened since run 639
- **Devnet verification post-rebrand: SUCCESS** — finalized_epoch=8 at slot 81, clean Gloas fork transition at epoch 1, 4 nodes healthy
- Explored codebase for actionable improvements: `can_builder_cover_bid` and `is_parent_block_full` correctly remain `pub` (cross-crate usage), bid clone at line 199 is necessary (ref to owned), no dead code or stale TODOs in Gloas code
- CI: all green, Docker build cache cleared and rebuild succeeded
- No code changes this run — spec stable, fully compliant, devnet verified

### 2026-03-09 — spec scan (run 639)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged or opened since run 638
- CI: latest ci run passed (all jobs), nightly in progress, 5+ consecutive green (Mar 5-9)
- Docker workflow: multiple cancelled runs from rapid pushes (normal), latest queued
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan (run 638)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged since run 637
- Notable: PRs #4984 (Verkle/EIP-6800) and #4977 (SSLE/EIP-7441) removed from specs as stagnant — no Gloas impact
- CI: latest ci run passed (all jobs), nightly 4+ consecutive green (Mar 5-8), today's nightly in progress
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan + codebase health check (run 637)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged or opened since run 636
- Codebase health: zero compiler warnings, zero clippy warnings, cargo audit clean (1 known rsa timing sidechannel, no fix available)
- CI: latest ci run passed (all 6 jobs), nightly 8+ consecutive green (Mar 4-9+, today's in progress — only http-api tests remaining)
- Reviewed all #[allow(dead_code)] annotations — all are standard Rust idiom (error enum context fields), no removable instances
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan (run 636)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged or opened since run 635
- Verified PR #4926 (SECONDS_PER_SLOT removal, merged Mar 2) — already handled: chain_spec.rs:2647-2662 has forward-compat logic deriving seconds_per_slot from SLOT_DURATION_MS when absent, with test at line 3741
- CI: latest ci run passed (all 6 jobs), nightly 7 consecutive green (Mar 4-9+)
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan (run 635)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged or opened since run 634
- PTC Lookbehind (#4979): design still not converged — 3 competing approaches (full 2-epoch cache, minimal size-1/2 cache, per-slot update in process_block). Bug is agreed-upon but community debating state size vs complexity trade-off. Not ready to pre-implement.
- CI: run 626 passed (all 6 jobs), nightly 6 consecutive green (Mar 4-9)
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan (run 634)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged or opened since run 633
- CI: latest completed run passed (run 626), nightly 5 consecutive green (Mar 5-9 in progress)
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan + upcoming PR pre-analysis (run 633)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged or opened since run 632
- CI (run 22844693747): all 6 jobs passed (success)
- Nightly: 5 consecutive green (Mar 4-8), today's queued
- Pre-analyzed upcoming PRs for readiness:
  - **#4898** (remove pending from tiebreaker): vibehouse already compliant — our `get_payload_tiebreaker` doesn't special-case PENDING, falls through to `should_extend_payload`. No code change needed.
  - **#4892** (remove impossible branch in is_supporting_vote): vibehouse already compliant — we use `==` check at line 1482 with assert comment. No code change needed.
  - **#4954** (fork choice store milliseconds): NO code change needed — vibehouse already stores time as `Slot` (not UNIX timestamp) and uses ms `Duration` for timeliness. Only EF test handler `on_tick` deserialization will need updating when test vectors change.
  - **#4843** (variable PTC deadline): renames `payload_present`→`payload_timely`, `is_payload_timely`→`has_payload_quorum`, adds `payload_envelopes` to store, `MIN_PAYLOAD_DUE_BPS` config, variable deadline by payload size. Last updated Feb 19, still evolving.
- Workspace tests: 2600/2609 passed, 8 failed (all web3signer_tests — external Java service timeout, not code regression)

### 2026-03-09 — spec scan (run 632)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged or opened since run 631
- CI (run 22844693747): 5/6 passed, beacon_chain tests still running (normal ~50min)
- Nightly: 5 consecutive green (Mar 4-8)
- Clean release build: zero warnings
- PTC Lookbehind (#4979): discussion ongoing re: assert placement in `get_ptc`, design settling on 2*SLOTS_PER_EPOCH
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan (run 631)
- All 10 tracked Gloas PRs still OPEN: #4979, #4960, #4954, #4940, #4932, #4898, #4892, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged since run 630
- CI: 4/6 passed, 2 in progress (normal)
- Nightly: 3 consecutive green (Mar 6-8)
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan + maintenance check (run 629)
- All 7 tracked Gloas PRs still OPEN: #4979, #4960, #4940, #4932, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged since run 628
- CI (run 22844693747): 4/6 jobs passed (check+clippy+fmt, ef-tests, network+op_pool), 3 still running (unit tests, http_api, beacon_chain)
- Nightly tests: 5 consecutive green runs (Mar 4-8)
- `cargo audit`: 1 known vulnerability (rsa timing sidechannel via jsonwebtoken — no fix available, not consensus-critical)
- `cargo outdated`: only 3 minor deps outdated (rand_xorshift 0.4→0.5 in types, rand/rand_chacha dev deps in network)
- All 67 TODO/FIXME comments in codebase are inherited non-actionable items (DAS sync, electra cleanup, mock builder quirks)
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan + code quality review (run 628)
- All 7 tracked Gloas PRs still OPEN: #4979, #4960, #4940, #4932, #4843, #4840, #4630
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged since run 627
- CI (run 22844693747): 4/6 jobs passed, 2 still running
- Deep code quality review: all Gloas ePBS functions use correct reference/ownership, no unnecessary clones in hot paths, no dead code, no unreachable branches
- Clean build: zero warnings
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan + security audit (run 627)
- All 6 key Gloas PRs still OPEN: #4979, #4940, #4932, #4960, #4840, #4939
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged — recent merges all CI/dep updates (#4991, #4990, #4988)
- PTC Lookbehind (#4979) design still evolving: potuz favors 2*SLOTS_PER_EPOCH, ensi321 proposes size-2, discussion ongoing about lookahead needs
- CI (run 22844693747): check+clippy+fmt passed, 5 test jobs in progress
- Nightly tests: 5 consecutive green runs (Mar 4-8)
- Full security audit of Gloas consensus code: zero `.unwrap()` in production paths, all arithmetic uses safe_arith traits, all index access bounds-checked, no unsafe casts
- `cargo audit`: 1 vulnerability (rsa timing sidechannel via jsonwebtoken, no fix available upstream — Engine API JWT, not consensus-critical)
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan + code quality audit (run 626)
- All 9 tracked PRs still OPEN: #4979, #4940, #4932, #4960, #4962, #4840, #4843, #4630, #4939
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs opened or merged since run 625
- Zero activity on any tracked PR since Mar 7
- CI for rebranding commits (run 22844187057): check+clippy+fmt + ef-tests passed, 4 test jobs still running
- Nightly tests: 6 consecutive green runs (Mar 4-9)
- Audited consensus-critical code for `.unwrap()` calls: state_processing, fork_choice, proto_array all clean — zero production unwraps
- Cleaned up 5 stale FIXME comments in execution_layer test_utils (inherited boilerplate, not actionable)

### 2026-03-09 — spec scan + CI monitoring (run 625)
- All 9 tracked PRs still OPEN: #4979, #4940, #4932, #4960, #4962, #4840, #4843, #4630, #4939
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs merged — recent merges all housekeeping (#4991, #4990, #4988, #4984 EIP-6800 Verkle removal)
- CI for rebranding commits (run 22844187057): check+clippy+fmt passed, 5 test jobs still running
- Nightly tests: 6 consecutive green runs (Mar 4-9)
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan (run 624)
- All 9 tracked PRs still OPEN: #4979, #4940, #4932, #4960, #4962, #4840, #4843, #4630, #4939
- No new consensus-specs release (still v1.7.0-alpha.2)
- No new Gloas-related PRs opened or merged since last scan
- Zero activity on any tracked PR since Mar 7
- CI for rebranding commits (run 22844187057) in progress — all 6 jobs running
- Nightly tests: 5 consecutive green runs (Mar 4-8)
- No code changes this run — spec stable, fully compliant

### 2026-03-09 — spec scan + CI verification (run 623)
- All 9 tracked PRs still OPEN: #4979, #4940, #4932, #4960, #4962, #4840, #4843, #4630, #4939
- No new consensus-specs release (still v1.7.0-alpha.2)
- Only 2 PRs merged since last scan (#4990, #4991): CI strategy matrix + release-drafter update — zero Gloas impact
- Note: #4979 (PTC Lookbehind) now also has `heze` label, suggesting it may be promoted to the next fork
- CI check+clippy+fmt passed for rebranding commits; remaining test jobs in progress
- Rebranding audit: only 2 Rust files contain "lighthouse" — both are P2P peer identification (ClientKind::Lighthouse, ClientCode::Lighthouse) for recognizing other Lighthouse clients on the network, correctly kept
- All kurtosis `cl_type: lighthouse` references correctly kept (infrastructure constraint)
- Nightly tests: 5 consecutive green runs (Mar 4-8)

### 2026-03-09 — spec scan (run 622)
- All 9 tracked PRs still OPEN: #4979, #4940, #4932, #4960, #4962, #4840, #4843, #4630, #4939
- No new consensus-specs release (still v1.7.0-alpha.2)
- Recent merged PRs (#4980-#4991) are all housekeeping: dependency updates, CI fixes, EIP-6800 removal — nothing Gloas-related
- Codebase clean: zero warnings, zero clippy issues, rebranding complete
- CI running for latest push (run 621 rebranding commits)

### 2026-03-09 — spec scan + CI cleanup (run 621)
- All 9 tracked PRs still OPEN: #4979, #4940, #4932, #4960, #4962, #4840, #4843, #4630, #4939
- No new consensus-specs release (still v1.7.0-alpha.2)
- Confirmed 3 additional merged PRs already implemented: #4918 (attestation validation), #4923 (parent payload unknown), #4931 (FOCIL — Heze fork, not relevant)
- Removed stale upstream workflow files: test-suite.yml, local-testnet.yml, book.yml, linkcheck.yml, mergify.yml (security: auto-approved sigp/lighthouse team PRs)
- Updated docker.yml to target `main` branch (was `unstable`/`stable`)
- Updated release.yml to remove Sigma Prime-specific content (testing checklist, upstream book URLs, PGP key reference)
- CI: check+clippy+fmt passing, other jobs in progress

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

### 2026-03-09 — spec scan + user-facing doc rebrand (run 619)
- All 9 tracked PRs still OPEN: #4979, #4940, #4932, #4960, #4962, #4840, #4843, #4630, #4939
- No new consensus-specs release (still v1.7.0-alpha.2)
- Updated user-facing docs: README.md (replaced stale "upstream tracking" section with "Heritage"), SECURITY.md (pointed to vibehouse repo), book intro/UI pages (rebranded to vibehouse), Cross.toml (removed stale upstream issue link)
- CI check+clippy+fmt passed, full lint clean

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
