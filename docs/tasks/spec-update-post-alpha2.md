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

**vibehouse has the same bug** — our `get_ptc_committee` (gloas.rs:377) computes PTC from scratch using current state balances. Will fix when one of the PRs merges.

### PR #4979 (original, potuz, Mar 4) — large cache approach
- `ptc_lookbehind: Vector[Vector[ValidatorIndex, PTC_SIZE], 2 * SLOTS_PER_EPOCH]` (~256KB per state)
- Caches all PTC committees for previous + current epoch
- Epoch processing shifts window and pre-computes next epoch
- 10 review comments, design debate ongoing

### PR #4992 (alternative, potuz, Mar 9) — minimal cache approach ← SIMPLER
- `ptc_lookbehind: Vector[Vector[ValidatorIndex, PTC_SIZE], 2]` (~8KB per state)
- Only caches 2 committees: previous slot and current slot
- Updated at every slot transition in `process_slots`: `state.ptc_lookbehind = [state.ptc_lookbehind[1], compute_ptc(state)]`
- `get_ptc(state, slot)` asserts `slot == state.slot or slot + 1 == state.slot`, returns from cache
- Much simpler: no epoch processing step, no fork upgrade initialization of full array
- Just opened Mar 9, no reviews yet

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

### 2026-03-09 — spec scan (run 700)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI run 690: check+clippy+ef-tests+network passed, beacon_chain+http_api+unit tests running
- Clippy: zero warnings, codebase clean
- No code changes needed

### 2026-03-09 — spec scan (run 699)
- All 13 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI: latest run in progress, previous run green (success)
- No code changes needed

### 2026-03-09 — spec scan (run 697)
- All 13 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI: run 690 in progress (4 jobs queued/running), run 680 green
- No code changes needed

### 2026-03-09 — spec scan (run 696)
- All 13 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI: run 690 in progress, run 680 green
- No code changes needed

### 2026-03-09 — spec scan (run 695)
- All 8 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI: last run green, current run in progress
- cargo audit unchanged, 0 dep updates
- No code changes needed

### 2026-03-09 — spec scan (run 693)
- All 13 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release
- CI green (latest ci run success), nightly 5+ consecutive greens
- cargo audit unchanged (1 medium rsa advisory, transitive)
- 0 dependency updates available
- No code changes needed

### 2026-03-09 — spec scan (run 691)
- All 13 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- PR #4992 (PTC Lookbehind minimal): updated today (15:35 UTC), still blocked, no external reviews
- Recently merged PRs: #4990 release-drafter, #4991 CI matrix — maintenance only
- CI in progress (all 6 jobs running from run 690 push)
- No code changes needed

### 2026-03-09 — spec scan (run 690)
- All 13 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- PR #4992 (PTC Lookbehind minimal): updated today, potuz self-review comments, still REVIEW_REQUIRED
- PR #4979 (PTC Lookbehind large): last updated Mar 7, REVIEW_REQUIRED
- Committed lockfile update: windows-sys 0.61.2 consolidation, syn 2.0.117 (transitive deps)
- CI green, cargo audit unchanged
- No spec code changes needed

### 2026-03-09 — spec scan (run 689)
- All 13 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- PR #4992 (PTC Lookbehind minimal): potuz left 2 review comments, CI failing on Gloas tests — still in flux
- PR #4979 (PTC Lookbehind large): active discussion Mar 6-7, CI green — may be closer to merge
- PRs #4898, #4892: have approvals but stalled in design discussion
- CI green: ci passed, nightly 3+ consecutive greens; clippy clean (zero warnings)
- cargo audit unchanged (1 medium rsa advisory, transitive)
- No code changes needed

### 2026-03-09 — spec scan (run 688)
- All 13 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- Recently merged PRs: #4990 release-drafter, #4991 CI matrix — maintenance only, nothing Gloas
- PR #4992 (PTC Lookbehind minimal): still 2 review comments, no external reviews, mergeable_state=blocked
- CI green: ci passed; clippy clean (zero warnings)
- All deps at latest compatible versions, cargo audit unchanged
- No code changes needed

### 2026-03-09 — spec scan (run 687)
- All 13 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- Recently merged PRs: #4990 release-drafter, #4991 CI matrix — maintenance only, nothing Gloas
- PR #4992 (PTC Lookbehind minimal): still 2 self-review comments from potuz, no external reviews yet
- CI green: ci passed; nightly 5+ consecutive greens
- No code changes needed

### 2026-03-09 — spec scan (run 686)
- All 13 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- PTC Lookbehind PR #4992 (minimal approach) has new self-review comments from potuz (naming: `previous_ptc/current_ptc`, placement in `process_slot`)
- CI in progress: 4/6 jobs passed (check+clippy+fmt, ef-tests, http_api, network+op_pool), beacon_chain + unit tests running
- Nightly: 5 consecutive greens (Mar 5-9)
- No code changes needed

### 2026-03-09 — spec scan (run 685)
- All 13 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- CI: check+clippy+fmt, ef-tests, http_api, network+op_pool passed; beacon_chain+unit tests in progress
- Clippy clean (zero warnings), cargo audit unchanged (1 medium rsa advisory, transitive)
- Full test coverage audit via agent: no gaps found — all error paths, edge cases, and integration points covered
- No code changes needed

### 2026-03-09 — spec scan (run 674)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (latest tagged: v1.6.1, Gloas spec: v1.7.0-alpha.2)
- CI in progress for run 671 push (check+clippy+ef_tests passed)
- cargo audit: unchanged (1 medium rsa advisory, transitive)
- No code changes needed

### 2026-03-09 — spec scan + edge case tests (run 671)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (latest tagged: v1.6.1, Gloas spec: v1.7.0-alpha.2)
- CI green, clippy clean
- Added 3 edge case tests for bid overwrite behavior:
  - `two_bids_different_builders_second_overwrites_payment`: verifies builder_index/amount/fee_recipient change when a different builder outbids
  - `same_builder_rebid_resets_ptc_weight`: verifies PTC weight is cleared on rebid (stale attestations)
  - `late_bid_after_envelope_preserves_payload_state`: verifies late bid doesn't reset payload_revealed
- All 677 state_processing + fork_choice tests pass

### 2026-03-09 — spec scan (run 670)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (latest tagged: v1.6.1, Gloas spec: v1.7.0-alpha.2)
- Recently merged PRs: all maintenance (#4991 CI matrix, #4990 release-drafter) — nothing Gloas
- CI green: ci passed, spec-test-version-check passed
- No code changes needed

### 2026-03-09 — spec scan (run 669)
- All 9 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (latest tagged: v1.6.1, Gloas spec: v1.7.0-alpha.2)
- Recently merged PRs: all maintenance — nothing Gloas
- CI green: ci passed, clippy clean, cargo audit unchanged
- No code changes needed

### 2026-03-09 — spec scan (run 668)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (latest tagged: v1.6.1, Gloas spec: v1.7.0-alpha.2)
- Recently merged PRs: all maintenance (#4991 CI matrix, #4990 release-drafter, #4988 test fix, #4986 renovate, #4985 deps, #4984 EIP-6800 removal, #4983 release-drafter, #4982 ruff, #4981 codespell, #4980 python) — nothing Gloas
- CI green: ci passed, spec-test-version-check passed
- Clippy clean, no production TODO/FIXME/HACK items
- No code changes needed

### 2026-03-09 — spec scan + conformance audit (run 667)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- CI green: ci passed, nightly 3 consecutive greens
- Deep spec conformance audit of critical functions: `is_parent_block_full`, `can_builder_cover_bid`, `process_payload_attestation` PTC membership, envelope verification ordering, `on_payload_attestation` quorum — all correct
- Test coverage audit: all Gloas modules thoroughly tested (envelope_processing 80+ tests/3372 lines, gloas.rs integration 20658 lines, upgrade/gloas.rs 32 tests, epoch processing 19 tests, block replayer 32 tests, fork choice validation 7+ tests)
- No code changes needed — codebase in excellent shape

### 2026-03-09 — spec scan (run 660)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- CI green: all 7 jobs passed (ef-tests, network+op_pool, http_api, beacon_chain, unit tests, check+clippy+fmt, ci-success)
- Nightly: 5 consecutive greens (Mar 5-9)
- No code changes needed

### 2026-03-09 — spec scan (run 654)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- CI green: ci passed, nightly passed, spec-test-version-check passed
- PTC Lookbehind (#4979): active review discussion Mar 6-7, design still in flux
- No code changes needed

### 2026-03-09 — spec scan (run 653)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- CI green: ci passed, nightly 5 consecutive greens (Mar 5-9)
- cargo audit: same 1 medium rsa advisory (transitive, no fix available)
- No code changes needed

### 2026-03-09 — spec scan (run 652)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- No new consensus-specs release (still v1.7.0-alpha.2)
- CI green: ci passed, nightly 5 consecutive greens (Mar 5-9)
- cargo audit: same 1 medium rsa advisory (transitive, no fix available)
- cargo outdated: only minor dev dep updates (rand), no actionable items
- No code changes needed

### 2026-03-09 — spec scan (run 651)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- 3 previously untracked PRs (#4954, #4898, #4892) confirmed still open
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- Recent merged PRs: all maintenance (#4990 release-drafter, #4991 CI matrix, #4988 test fix, #4985 deps)
- CI green: ci passed, nightly 4 consecutive greens (Mar 7-9)
- No code changes needed

### 2026-03-09 — spec scan (run 650)
- All 12 tracked Gloas PRs still OPEN (no merges since last scan)
- Found 2 Gloas-labeled PRs not in our tracker: #4747 (Fast Confirmation Rule, eip7805) and #4558 (Cell Dissemination) — neither is core ePBS, no implementation needed
- Recent merged PRs: all maintenance (dep updates, CI, EIP-6800/7441 removal) — nothing Gloas
- No new consensus-specs release (still v1.7.0-alpha.2), no new spec-test release (still v1.5.0)
- CI all green, cargo audit unchanged
- No code changes needed

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
