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

## Upcoming: PTC Lookbehind (competing PRs #4979 and #4992, both OPEN)

Fixes a real bug: when processing payload attestations at epoch boundaries (e.g., slot 32 validating PTC of slot 31), effective balance changes from epoch processing can cause `get_ptc` to return a different committee than what was valid when the attestation was created.

**vibehouse has the same bug** — our `compute_ptc` (gloas.rs) computes PTC from scratch using current state balances. **Implementation ready on branch `ptc-lookbehind`** — adds `previous_ptc`/`current_ptc` to BeaconState, rotates in per_slot_processing, all 575 unit tests pass. Blocks on PR #4992 merge + new spec test vectors (SSZ layout change).

### PR #4979 (original, potuz, Mar 4) — large cache approach — CLOSED (rejected)
- `ptc_lookbehind: Vector[Vector[ValidatorIndex, PTC_SIZE], 2 * SLOTS_PER_EPOCH]` (~256KB per state)
- Caches all PTC committees for previous + current epoch
- Closed in favor of #4992 (minimal approach)

### PR #4992 (alternative, potuz, Mar 9, actively updated Mar 10) — minimal cache approach ← SIMPLER
- `ptc_lookbehind: Vector[Vector[ValidatorIndex, PTC_SIZE], 2]` (~8KB per state)
- Only caches 2 committees: previous slot (`[0]`) and current slot (`[1]`)
- Updated at every slot transition in `process_slots` (after slot increment, after epoch processing): `state.ptc_lookbehind = [state.ptc_lookbehind[1], compute_ptc(state)]`
- New `compute_ptc(state)` extracted helper — computes PTC for `state.slot` using seed + beacon committees + balance-weighted selection
- `get_ptc(state, slot)` asserts `slot == state.slot or slot + 1 == state.slot`, returns `[1]` for current, `[0]` for previous
- Fork upgrade: initialize `[[0; PTC_SIZE], [0; PTC_SIZE]]`, then compute `[1]` after builder onboarding
- `get_ptc_assignment` removed entirely from validator spec (validators just check `get_ptc` for current slot)
- Fork choice: reorder `on_payload_attestation_message` to check slot before calling `get_ptc` (assertion safety)
- Active review: potuz addressing feedback, design converging

**Implementation plan (for whichever merges):**
1. New BeaconState field: `ptc_lookbehind` (size depends on which PR wins)
2. Extract `compute_ptc` from current `get_ptc_committee`
3. Refactor `get_ptc` to read from cache
4. If #4992: update `process_slots` to shift cache each slot
5. If #4979: add epoch processing step + fork upgrade initialization
6. Update callers: `process_payload_attestation`, `get_indexed_payload_attestation`, `validator_ptc_duties`

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

## DONE: Index-1 Attestation Envelope Validation (PR #4939, still OPEN — implemented proactively)

Adds two new gossip validation rules for `beacon_aggregate_and_proof` and `beacon_attestation_{subnet_id}`:
1. **[REJECT]** If `data.index == 1` (payload present for past block), the execution payload for that block must have passed validation
2. **[IGNORE]** If `data.index == 1`, the execution payload envelope must have been seen. Clients MAY queue the attestation and SHOULD request the envelope via `ExecutionPayloadEnvelopesByRoot`

**Implemented in run 701:** Added `verify_payload_envelope_for_index1()` in `attestation_verification.rs`. Checks `head_block.envelope_received` (IGNORE if not seen) and `head_block.execution_status` (REJECT if Invalid). Applied to both aggregate and unaggregated attestation gossip validation. New error variants: `PayloadEnvelopeNotSeen`, `PayloadNotValidated`. Gossip handler maps to `MessageAcceptance::Ignore` and `MessageAcceptance::Reject` respectively. Existing test updated to verify gossip-level rejection.

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

### run 946 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged head d76a278b0a, 1 APPROVED (jtraglia Mar 12). Still OPEN, mergeable=clean.
- **PR #4939 (index-1 attestation)**: updated Mar 13 (head fdfad73e31), still OPEN, mergeable=blocked. Already implemented proactively.
- All other tracked PRs (#5002, #4954, #4843, #4898, #4892, #4940, #4932, #4960, #4962): all still OPEN.
- CI run 23035917712: SUCCESS. 0 compatible dep updates. cargo audit unchanged (1 vuln + 5 allowed). No code changes needed.

### run 945 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged head d76a278b0a, 1 APPROVED (jtraglia Mar 12). Still OPEN.
- All other tracked PRs (#5002, #4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962): all still OPEN.
- CI run 23035917712: SUCCESS. 0 compatible dep updates. cargo audit unchanged. No code changes needed.

### run 944 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged head d76a278b0a, 1 APPROVED (jtraglia Mar 12). Still OPEN.
- **PR #4940 (fork choice tests)**: rebased Mar 12 (renamed test files), head b75963292f, mergeable=true. Still OPEN, no reviews.
- All other tracked PRs (#5002, #4954, #4843, #4898, #4892, #4939, #4932, #4960, #4962): all still OPEN.
- CI run 23035917712: SUCCESS. Clippy clean. 0 compatible dep updates. cargo audit unchanged. No code changes needed.

### run 943 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged head d76a278b0a, 1 APPROVED (jtraglia Mar 12). Still OPEN. Title changed to "Add cached PTCs to the state".
- All other tracked PRs (#5002, #4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962): all still OPEN.
- CI run 23035917712: SUCCESS (all 7 jobs passed). 0 compatible dep updates. cargo audit unchanged. No code changes needed.

### run 939 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged head d76a278b0a, 1 APPROVED (jtraglia Mar 12). Still OPEN.
- All other tracked PRs (#5002, #4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962): all still OPEN.
- CI run 23035917712: 4/6 passed (check+clippy, EF tests, http_api, network+op_pool), unit+beacon_chain in_progress. 0 compatible dep updates. No code changes needed.

### run 938 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged head d76a278b0a, mergeable=true, 1 APPROVED (jtraglia Mar 12). Still OPEN.
- **PR #5002 (self-build wording)**: 1 APPROVED (jtraglia Mar 13). Documentation-only, no consensus impact.
- All other tracked PRs (#4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962): all still OPEN.
- Clippy clean (verified locally). CI run 23035917712 in_progress. 0 compatible dep updates. No code changes needed.

### run 931 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged head d76a278b0a, mergeable=true, 1 APPROVED (jtraglia Mar 12). Still OPEN.
- All other tracked PRs (#4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962): all still OPEN.
- CI in_progress for commit 2f3aaf9 (check+clippy passed, other jobs running). 0 compatible dep updates. No code changes needed.

### run 930 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged head d76a278b0a, mergeable=clean, 1 APPROVED (jtraglia Mar 12). Still OPEN — closest to merge.
- All other tracked PRs (#4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962): all still OPEN.
- CI in_progress. No code changes needed.

### run 928 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged head d76a278b0a, 8 commits, mergeable=true, 1 APPROVED (jtraglia Mar 12). jtraglia making minor style changes (Mar 12 comments). Closest to merge of all tracked PRs.
- **PR #4939 (index-1 attestation envelope)**: still OPEN, updated Mar 13.
- CI in_progress for commit 43e6e91 (community PR #30 merge). cargo audit unchanged (1 vuln + 5 allowed). 0 compatible dep updates.
- Other tracked PRs (#4954, #4843, #4898, #4892, #4940, #4932, #4960, #4962): all still OPEN.
- No code changes needed.

### run 927 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind)**: unchanged head d76a278b0a, 8 commits, mergeable=true, 1 approval. jtraglia actively reviewing (Mar 11-12 comments on initialization syntax). Approaching merge.
- **Merged community PR #30**: Rust 1.88→1.91 + Bullseye→Trixie Docker base (barnabasbusa). Required because alloy-consensus 1.7.3 needs rustc 1.91+, redb 3.1.1 needs 1.89+.
- Other tracked PRs (#4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962): all still OPEN.
- CI in_progress for run 926 commit. No compatible dep updates. cargo audit unchanged (1 vuln + 5 allowed).

### run 926 (Mar 13)
- Spec scan: no new consensus-specs merges since #5001 (Mar 12). No new spec release (v1.7.0-alpha.3 committed but not released on GitHub, still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0).
- **PR #4992 (PTC lookbehind) major evolution**: now 8 commits (was 2), head d76a278b0a (was 215962a9), **mergeable=clean** (was blocked). Design changed: split single `ptc_lookbehind` vector into two separate BeaconState fields `previous_ptc`/`current_ptc`. New `compute_ptc` helper extracted. `get_ptc` now reads from cached fields with slot assertion. `process_slots` rotates cache each slot (after epoch processing + slot increment). `get_ptc_assignment` removed from validator spec. Fork upgrade: zeros + compute after builder onboarding. New tests: ptc rotation, epoch boundary crossing. Approaching merge readiness.
- **New PR #5002**: "Make wordings clearer for self build payload signature verification" (ensi321, p2p-interface.md only, 2 line wording change, no consensus impact).
- Other tracked PRs (#4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962): all still OPEN.
- Updated dependencies: clap 4.5→4.6, openssl 0.10.75→0.10.76, c-kzg 2.1.6→2.1.7, tempfile 3.26→3.27, once_cell 1.21.3→1.21.4, anstream 0.6→1.0. Build clean, types 715/715 pass.
- CI in_progress for latest commit. Nightly green. cargo audit unchanged (1 vuln + 5 allowed).

### run 925 (Mar 13)
- **Spec change implemented: PR #5001** — "Add parent_block_root to bid filtering key" (merged since last run). Changed bid highest-value gossip filtering from `(slot, parent_block_hash)` to `(slot, parent_block_hash, parent_block_root)`. Prevents cross-fork bid interference when competing beacon chain forks share the same execution parent. Updated `is_highest_value_bid()` signature, caller in `gloas_verification.rs`, and all tests. New test `highest_value_different_parent_root_independent` added.
- Version bump to v1.7.0-alpha.3 (#4999) committed to consensus-specs but not yet released as a GitHub release.
- PR #4979 (PTC lookbehind large cache) CLOSED without merge — #4992 (minimal 2-slot cache) is the surviving approach, still open.
- Other tracked PRs (#4992, #4954, #4843, #4898, #4892, #4939, #4940, #4932, #4960, #4962): all still OPEN.
- No new spec-test vectors (still v1.6.0-beta.0). CI green, nightly green.
- All tests pass: observed_execution_bids 19/19, bid verification integration 4/4, network bid gossip 18/18, HTTP API bid 16/16.

### run 922 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): updated today, still open. CI green, nightly green. No code changes needed.

### run 921 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, blocked). CI green, nightly green. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 920 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): updated today but still open, design converging. CI green. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 919 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, last activity Mar 10 12:05 UTC). CI green (all 7 jobs passed). 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 918 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, last activity Mar 10 12:05 UTC). CI green. Scheduled nightly failed on stale commit 0d12a85 (pre-race-fix); manual nightly on 7e2ab4d passed — not a real regression. 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 917 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, last activity Mar 10 12:05 UTC). CI green, nightly green. 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 914 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, last activity Mar 10 12:05 UTC). CI green, nightly green. 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 913 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, last activity Mar 10 12:05 UTC). CI green, nightly green. 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 912 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, last activity Mar 10 12:05 UTC). CI green, nightly green. 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 911 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, blocked). Scheduled nightly failed on stale commit 0d12a85 (before data_column_reconstruction race fix 62df568); manual nightly running on 7e2ab4d (includes fix). 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 910 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, no new comments since Mar 10 12:05 UTC). CI green, nightly in progress. 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 908 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, no new comments since Mar 10 12:05 UTC). CI green, nightly in progress (http-api-tests fulu/electra running, all 24 other jobs passed). 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 907 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): unchanged (head 215962a9, blocked, no new comments). CI green, nightly in progress (http-api-tests fulu/electra running, all 24 other jobs passed). 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 906 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 (PTC lookbehind): no new activity since potuz Mar 10 12:05 UTC. CI green, nightly in progress (http-api-tests fulu/electra running, all 23 other jobs passed). 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 905 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). CI green, nightly in progress (http-api remaining, all else passed). 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 904 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). CI green, nightly in progress. 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 903 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). CI green, nightly in progress. 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 902 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PTC lookbehind debate unchanged — no new comments since potuz Mar 10 12:05 UTC. CI green, nightly in progress. 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 901 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992: ensi321 commented preferring #4992 over #4979, potuz responded defending full lookahead approach — debate continues. CI green, nightly in progress. 0 compatible dep updates. cargo audit: 1 vuln + 5 allowed (unchanged). No code changes needed.

### run 900 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 unchanged (2 commits, last comment potuz Mar 10). CI green, nightly in progress. 0 compatible dep updates. cargo audit: same 6 advisories (rsa medium + 5 allowed). No code changes needed.

### run 899 (Mar 10)
- Spec scan: no new consensus-specs commits (latest: #4995 python 3.14 support, Mar 10). All 11 tracked Gloas PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). PR #4992 unchanged (2 commits, last comment kevaundray Mar 9). CI green, nightly in progress. 0 compatible dep updates. cargo audit: same 6 advisories (rsa medium + 5 allowed). No code changes needed.

### run 898 (Mar 10)
- Spec scan: no new consensus-specs commits. All 12 tracked PRs still OPEN. No new spec release (still v1.7.0-alpha.2), no new spec-test vectors. PR #4992 unchanged. CI green, nightly running. 0 dep updates. No code changes needed.

### 2026-03-10 — consolidated: runs 746-804 (Mar 10)
Spec stable throughout. All 11 tracked Gloas PRs remain OPEN. No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.5.0 stable, v1.6.0-beta.0 latest pre-release — no Gloas). CI continuously green, nightly continuously green (7 consecutive green runs Mar 5-10). PTC lookbehind #4992: potuz pushed commit 215962a (Mar 10 01:05 UTC) removing `get_ptc_assignment` entirely and addressing fork upgrade; design converging but still needs approvals. Recent spec commits are all CI/tooling/cleanup — no consensus changes.

Runs 801-897 (Mar 10): Spec completely stable. No new merges, no new release (still v1.7.0-alpha.2), no new spec test vectors (still v1.6.0-beta.0). All 11 tracked Gloas PRs remain OPEN. PR #4992: potuz commented (Mar 10 12:05 UTC) defending #4979 full lookahead approach — debate ongoing, no approvals on either PR. Recently merged consensus-specs PRs (#4988, #4990-#4995): all CI/tooling/cleanup — no consensus changes. CI fully green. Nightly running on current HEAD. 0 compatible dep updates, 0 cargo audit changes.

Runs 832-895: Spec completely stable — no new merges, no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). All 11 tracked Gloas PRs remain OPEN. PR #4992 unchanged (commit 215962a). Recently merged consensus-specs PRs (#4984 remove Verkle, #4977 remove Whisk, #4994-#4995 Python/CI) — no consensus changes. CI fully green. Nightly Mar 10 failed on stale commit 0d12a85 (before data_column_reconstruction race fix 62df568); manual nightly triggered on current HEAD (queued). Updated schannel 0.1.28→0.1.29 (run 847). 0 compatible dep updates.
- Run 831: Spec scan (no changes). Race fix verified stable (5/5 local passes). CI running for fix commit.
- Run 829: Fixed remaining race in `data_column_reconstruction_at_deadline` — previous fix (run 826) broke out of event loop on reconstruction before all gossip events were drained; now collects both reconstruction AND all gossip events before breaking. 5/5 local runs pass.
- Run 826: Fixed flaky `data_column_reconstruction_at_deadline` nightly test (race condition in event ordering)
- Run 820: Committed Cargo.lock update (enr syn 1→syn 2)
- Run 810: Reviewed PR #4992 diff — implementation plan ready
- Run 804: Updated libz-sys 1.1.24→1.1.25
- Run 801: Audited PR #4906 test scenarios — all handled correctly, no changes needed

All tests green throughout: EF 138/138, workspace 2643/2643, clippy clean, CI and nightly continuously green.

Key verifications across these runs:
- EF spec tests: consistently 35/35 (minimal) and 138/138 (full fake_crypto) passing
- Fork choice EF tests: 8/8 passing (real crypto)
- Workspace tests: 2643/2643 passing
- Beacon chain tests: 768/768 passing (FORK_NAME=gloas)
- Devnet verification: finalized_epoch=8 (run 753)
- Zero compiler warnings, zero clippy warnings
- Cargo audit: 6 advisories (all unmaintained/transitive, no actionable vulns)
- No compatible dependency updates available
- Code quality audits: no unwraps in consensus paths, no actionable TODOs, all Gloas functions have test coverage
- Docker workflow queued throughout (self-hosted runner availability, not code issue)
- No code changes needed across all runs

### run 745 (Mar 9)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.5.0)
- PTC lookbehind PR #4992: 5 review comments from potuz/kevaundray; kevaundray raised `get_ptc_assignment` needing 32 slots vs 2-slot cache; potuz acknowledged duties functions not yet addressed. Design still evolving.
- CI fully green (run 22875542079): all 7 jobs passed including beacon_chain, http_api, network+op_pool
- Nightly tests: 4 consecutive green runs (Mar 6-9)
- Workspace tests: 2612/2612 passed (8 web3signer_tests timeout — external service, not code)
- Clippy clean, cargo audit unchanged (rsa advisory only, no fix available)
- Test coverage analysis: 366 Gloas integration tests in beacon_chain, comprehensive fork_choice/state_processing coverage. No actionable gaps found.

### run 742 (Mar 9)
- Spec scan: all 10 tracked PRs still OPEN, no new Gloas merges, no new spec release
- PTC lookbehind PR #4992: potuz acknowledged duties functions not yet addressed; design still evolving
- Recently merged consensus-specs PRs (#4990-4993): all CI/tooling/cleanup, no consensus changes
- Workspace tests: 2643/2647 passed (4 web3signer_tests timeout — external service, not code)
- Clippy clean, cargo audit unchanged (rsa advisory only)

### run 722 (Mar 9)
- Spec scan: all 10 tracked PRs still OPEN, no new Gloas merges
- PTC lookbehind PR #4992: active review discussion — kevaundray raised `get_ptc_assignment` needing 32 slots vs 2-slot cache; potuz acknowledged duties functions not yet addressed. Design still evolving, not ready to implement.
- CI (run 22869552009): 4/6 jobs passed (check+clippy+fmt, EF tests, network+op_pool), beacon_chain/unit/http_api still running
- Nightly tests: 3 consecutive green runs (Mar 7-9)
- Code safety audit: no unwrap()/expect() in production consensus code (state_processing, fork_choice, proto_array); all array indexing properly bounds-checked
- cargo audit: same known rsa advisory, no new issues
- Dependencies: all lockfile deps at latest compatible versions; 30 major-version bumps available but no compatible patches

### run 721 (Mar 9)
- Spec scan: all 10 tracked PRs still OPEN, no new Gloas merges, no new spec release
- Devnet verification SUCCESS: finalized_epoch=8, clean Gloas fork progression (run ID 20260309-195504)
- CI green: check+clippy+fmt passed, EF tests passed, remaining jobs in progress
- cargo audit: same known advisories (rsa medium, 5 allowed warnings), no new issues
- Production consensus code audit: no unwrap() calls in non-test state_processing paths

### run 720 (Mar 9)
- Fixed spec conformance bug in `process_execution_payload_envelope`: self-build envelopes (builder_index == BUILDER_INDEX_SELF_BUILD) were incorrectly going through signature verification when `VerifySignatures::True`. The spec's `is_valid_indexed_execution_payload_envelope` returns True immediately for self-build without checking the signature. In practice the beacon chain always passes `VerifySignatures::False`, but this would fail if EF tests ever include self-build envelope test vectors.
- Added `self_build_envelope_skips_signature_check` test verifying Signature::empty() passes with VerifySignatures::True
- Updated 3 existing tests that tested the old (incorrect) behavior to match the spec
- All 575 state_processing tests pass, all EF spec envelope tests pass, clippy clean
- Spec scan: no new merges, PTC lookbehind PRs (#4979, #4992) still open and in design debate

### 2026-03-09 — consolidated: runs 642-719 (Mar 9)
Spec stable throughout. All 10 tracked Gloas PRs remain OPEN. No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.5.0). CI and nightly continuously green.

Key activities:
- **run 719**: added PTC lookbehind bug demonstration test — proves `get_ptc_committee` returns different results for the same slot when effective balances change, documenting the known bug that PR #4992 fixes
- **run 718**: deep spec conformance audit — verified `compute_balance_weighted_selection` (proposer/sync/PTC), `process_execution_payload`, `process_builder_pending_payments`, `process_slot`, `process_epoch` ordering all match spec
- **run 717**: consolidated progress log (was 350+ lines of repeated "no changes" entries)
- **run 704**: corrected PR #4939 status — still OPEN, our implementation is proactive
- **run 701**: implemented index-1 attestation envelope validation (PR #4939) — added `verify_payload_envelope_for_index1()`, new error variants `PayloadEnvelopeNotSeen`/`PayloadNotValidated`, all tests pass
- **run 690**: committed lockfile update (windows-sys 0.61.2, syn 2.0.117)
- **run 685**: full test coverage audit — no gaps found
- **run 671**: added 3 edge case tests for bid overwrite behavior (different builders, rebid PTC reset, late bid preservation)
- **run 667**: deep spec conformance audit — all critical functions verified correct
- **run 643**: discovered 2 new tracked PRs (#4939, #4962), expanded tracker to 12

Notable PR activity observed:
- PR #4992 (PTC Lookbehind minimal): active review from potuz and kevaundray, design concerns about `get_ptc_assignment` needing 32 slots while cache only holds 2
- PR #4979 (PTC Lookbehind large): active discussion Mar 6-7, design debate ongoing
- PRs #4898, #4892: have approvals but stalled in design discussion
- cargo audit: 1 medium rsa advisory (transitive, no fix available)

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
