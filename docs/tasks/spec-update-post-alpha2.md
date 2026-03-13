# Spec Update: Post v1.7.0-alpha.2 Changes

## Objective
Implement Gloas spec changes merged to consensus-specs master after v1.7.0-alpha.2.

## Status: DONE (all changes already implemented — verified through v1.7.0-alpha.3)

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

## Upcoming: PTC Lookbehind (PR #4992, OPEN — 1 APPROVED, approaching merge)

Fixes a real bug: when processing payload attestations at epoch boundaries (e.g., slot 32 validating PTC of slot 31), effective balance changes from epoch processing can cause `get_ptc` to return a different committee than what was valid when the attestation was created.

**vibehouse has the same bug** — our `compute_ptc` (gloas.rs) computes PTC from scratch using current state balances. **Implementation ready on branch `ptc-lookbehind`** — adds `previous_ptc`/`current_ptc` to BeaconState, rotates in per_slot_processing, all 575 unit tests pass. Blocks on PR #4992 merge + new spec test vectors (SSZ layout change).

### PR #4979 (original, potuz, Mar 4) — large cache approach — CLOSED (rejected)
- Closed in favor of #4992 (minimal approach)

### PR #4992 (alternative, potuz, Mar 9) — minimal cache approach ← WINNING
- `previous_ptc`/`current_ptc`: two separate BeaconState fields (each `Vector[ValidatorIndex, PTC_SIZE]`, ~4KB each)
- Updated at every slot transition in `process_slots` (after slot increment, after epoch processing): rotate current→previous, compute new current
- New `compute_ptc(state)` extracted helper — computes PTC for `state.slot`
- `get_ptc(state, slot)` asserts `slot == state.slot or slot + 1 == state.slot`, returns current or previous
- Fork upgrade: initialize both to zeros, then compute current after builder onboarding
- `get_ptc_assignment` removed entirely from validator spec
- **Status**: head d76a278b0a, mergeable=clean, 1 APPROVED (jtraglia Mar 12). Approaching merge.

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

- **PR #4940** — "Add initial fork choice tests for Gloas": MERGED (Mar 13). Tests `on_execution_payload` (EMPTY→FULL transition), basic head tracking. Our `ForkChoiceHandler` already supports `on_execution_payload` steps and `head_payload_status` checks — ready to pass when spec-test vectors are released.
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

### run 1163 (Mar 13) — perf: reuse active_votes in proposer boost, eliminate double lookups in tiebreaker
Spec stable — v1.7.0-alpha.3 now published (no new code since #5004). PR #4992 still OPEN, NOT MERGED (1 APPROVED, same head d76a278b0a). No new spec-test vectors. No semver-compatible cargo updates.
Two optimizations: (1) `should_apply_proposer_boost_gloas` now receives the pre-filtered `active_votes` slice instead of re-iterating all validators with per-validator zero-root/zero-balance checks. (2) `get_payload_tiebreaker` resolves the proto_node once and passes it to `should_extend_payload`, eliminating redundant HashMap lookups that previously occurred in both functions. 188/188 proto_array + 119/119 fork_choice + 8/8 EF fork choice tests pass. Clippy clean.

### run 1162 (Mar 13) — perf: pre-filter active votes in find_head_gloas
Spec stable — no new consensus-specs commits since #5004. PR #4992 still OPEN, NOT MERGED (1 APPROVED, same head d76a278b0a). No new releases or spec-test vectors. No semver-compatible cargo updates.
Pre-compute active votes (non-zero root + non-zero balance) once before the find_head_gloas loop, then pass the filtered slice to `get_gloas_weight`. Eliminates per-validator zero-root checks, balance bounds checks, and zero-balance checks from the inner loop that runs at every depth level for every child node. On mainnet with ~1M validators, this avoids ~3 comparisons × N_inactive_validators × depth × children_per_level iterations. 188/188 proto_array + 119/119 fork_choice + 8/8 EF fork choice tests pass. Clippy clean.

### run 1161 (Mar 13) — perf: reuse ancestor cache allocation in find_head_gloas
Spec stable — no new consensus-specs commits since #5004. PR #4992 still OPEN, NOT MERGED (1 APPROVED, same head d76a278b0a). PR #5003 CLOSED by author (not merged). No new releases or spec-test vectors. No semver-compatible cargo updates.
Moved `ancestor_cache` HashMap allocation outside the `find_head_gloas` loop, using `clear()` between iterations instead of reallocating. Avoids ~30 HashMap heap allocations per find_head call (one per tree depth level). 188/188 proto_array + 119/119 fork_choice + 8/8 EF fork choice tests pass. Clippy clean.

### run 1152 (Mar 13) — perf: reduce repeated borrows in attestation flag updates
Spec stable — no new consensus-specs commits since #5004. PR #4992 still OPEN, NOT MERGED (1 APPROVED, same head d76a278b0a). No new releases or spec-test vectors. No semver-compatible cargo updates.
Optimized `process_attestation` inner loop (all post-Altair forks): restructured participation flag updates to call `get_epoch_participation_mut()` once per validator instead of 3 times (once per flag), and `get_base_reward()` once instead of up to 3. Uses a bitmask to track newly-set flags, then processes rewards in a second pass. Reduces superstruct match overhead on every attestation in every block. 575/575 state_processing tests + 35/35 EF operations/epoch/sanity tests pass. Clippy clean.

### run 1151 (Mar 13) — perf: eliminate HashSet in get_attesting_indices
Spec stable — no new consensus-specs commits since #5004. PR #4992 still OPEN, NOT MERGED. No new releases or spec-test vectors. No semver-compatible cargo updates.
Optimized `get_attesting_indices` (Electra/Gloas hot path): replaced per-committee `HashSet<u64>` with direct `Vec::push`, pre-allocated output Vec via `num_set_bits()`, and inlined committee_bits iteration. Removes HashSet allocation overhead on every attestation verification. 575/575 state_processing tests + 15/15 EF operations tests pass. Clippy clean.

### runs 1130-1147 (Mar 13) — all stable
Spec completely stable — latest commit #5004 (release notes tooling, no code). No new release (v1.7.0-alpha.3 version bump in master but no GitHub release published, latest published still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). CI green. Nightly green (3 consecutive). No dep updates available. No semver-compatible cargo updates. PR #4992 still OPEN, NOT MERGED, same head d76a278b0a, 1 APPROVED (jtraglia). All tracked spec PRs (#4932, #4939, #4960, #4962) unchanged.

### runs 1123-1129 (Mar 13) — all stable
Spec completely stable — latest commit #5004 (release notes tooling, no code). No new release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.6.0-beta.0). CI green. No dep updates available. PR #4992 still OPEN, NOT MERGED, same head d76a278b0a. PR #4939 got wording-only commit (2b9e66eca3 "Refactor sentences for clarity/consistency") — adds index-1 rules to aggregate section, no semantic changes, our implementation already covers both paths. All tracked spec PRs (#4932, #4939, #4960, #4962, #5003) unchanged in substance.

### run 1122 (Mar 13) — dep update, all stable
Spec completely stable — no new consensus-specs commits since #4940 (Mar 13). No new release. No new spec-test vectors. CI green. Updated derive-where 1.6.0→1.6.1. PR #4992 (PTC lookbehind) still OPEN, MERGEABLE, same head d76a278b0a. All tracked spec PRs (#4932, #4939, #4960, #4962, #5003) unchanged. cargo audit unchanged (1 rsa, 5 warnings).

### runs 1098-1121 (Mar 13) — all stable, devnet verified
Spec completely stable — no new consensus-specs commits since #4940 (Mar 13). No new release (still v1.7.0-alpha.2, v1.7.0-alpha.3 version bump in master but no GitHub release published). No new spec-test vectors (still v1.6.0-beta.0 release). CI green. Nightly green (3 consecutive). Clippy clean (zero warnings). cargo audit unchanged (1 rsa, 5 warnings). No semver dep updates available. Run 1119: devnet verified — finalized_epoch=8 at slot 80, clean progression through Gloas fork, all 4 nodes healthy. PR #4992 (PTC lookbehind) still OPEN, MERGEABLE, 1 APPROVED (jtraglia), same head d76a278b0a — jihoonsong review comments (Mar 13) noting `get_ptc_assignment` still referenced in spec. PR #5003 (simplify process_proposer_lookahead): new PR from jihoonsong, Fulu-only Python slice fix — no impact on vibehouse (our Rust impl uses explicit indexed writes). All tracked spec PRs (#4932, #4939, #4960, #4962, #5003) unchanged.

### runs 1058-1097 consolidated (Mar 13) — all stable, no code changes needed
Spec completely stable — no new consensus-specs commits since #4940 (Mar 13). No new release (still v1.7.0-alpha.2). No new spec-test vectors (still v1.5.0). CI and nightly continuously green. cargo audit unchanged (1 rsa, 5 warnings). No semver-compatible dep updates available.

Notable events:
- Run 1069: PR #4940 ("Add initial fork choice tests for Gloas") merged — test-only, our handler already supports the new steps
- Run 1058: PR #5002 ("Make wording clearer for payload signature verification") merged — wording-only, no code impact
- Run 1059: Rebased `ptc-lookbehind` onto main — 575/575 state_processing tests pass
- PR #4992 (PTC lookbehind): continuously OPEN, MERGEABLE, 1 APPROVED (jtraglia), same head d76a278b0a
- Tracked spec-test PRs (#4932, #4960) still OPEN
- Codebase audit (run 1089): zero clippy/compiler warnings, zero unwraps in production consensus code, no actionable TODOs

### runs 1055 + 1019-1054 consolidated (Mar 13) — all stable, ptc-lookbehind kept rebased
Spec completely stable — no new consensus-specs commits since #5001 (Mar 12). No new release (latest published: v1.7.0-alpha.2). No new spec-test vectors. All tracked spec-test PRs (#4932, #4939, #4940, #4960, #4962) still OPEN.

Notable activities:
- Run 1054: Committed Cargo.lock transitive dep update (windows-sys 0.61.2, syn 2).
- Runs 1020, 1033, 1044, 1047, 1050: Rebased `ptc-lookbehind` branch onto main (task-doc drift only). 575/575 state_processing tests pass each time.
- Run 1032: Full codebase audit — clippy clean, cargo doc clean, production unwrap audit (zero unwraps in consensus/state_processing prod code), 28 TODOs in beacon_node (all inherited DAS sync, none Gloas).
- Run 1026: Verified all unwraps in consensus/state_processing are test-only.
- Run 1019: Test coverage audit — withdrawal processing (~50 tests), epoch processing well-covered, no significant gaps.
- PR #4992 (PTC lookbehind): continuously OPEN, APPROVED, MERGEABLE. No new commits since d76a278b0a (Mar 12).
- PR #4939 (index-1 attestation): wording-only update Mar 13, no semantic changes — our implementation still matches.
- CI and nightly continuously green. cargo audit unchanged (1 rsa). No compatible dep updates.

### run 1018 (Mar 13) — dep update, code audit, all stable
- Updated tracing-subscriber 0.3.22→0.3.23. Build clean, 368/368 Gloas state_processing tests pass.
- Deep audit of envelope processing path (envelope_processing.rs, gloas_verification.rs, beacon_chain.rs self-build): no critical bugs, no panics, all arithmetic safe, spec-compliant.
- PR #4992 (PTC lookbehind): still OPEN, MERGEABLE, 1 APPROVED (jtraglia). Implementation ready on `ptc-lookbehind` branch.
- Spec stable — no new consensus-specs commits since #5001 (Mar 12). No new release or spec-test vectors.
- CI and nightly green. Clippy clean. cargo audit unchanged (1 rsa).

### runs 968-1017 consolidated (Mar 13) — all stable, no code changes needed
Spec completely stable — no new consensus-specs commits since #5001 (Mar 12). No new release (latest: v1.7.0-alpha.3). No new spec-test vectors (still v1.6.0-beta.0).
- Run 1005: Formal audit of v1.7.0-alpha.3 diffs — all 7 changes already implemented (PayloadStatus reorder, is_pending_validator, dual PTC voting, should_extend_payload DA requirement, index-1 validation, bid filtering, envelope serve range).
- Run 988: Added 5 SSZ round-trip tests for proto_array Gloas fields (ProtoNode, VoteTracker, SszContainer).
- PR #4992 (PTC lookbehind): continuously OPEN, MERGEABLE, 1 APPROVED (jtraglia). Implementation ready on `ptc-lookbehind` branch.
- PR #5002 (wording-only): 1 APPROVED (jtraglia). No code impact.
- CI and nightly continuously green. cargo audit unchanged (1 rsa). No compatible dep updates available.

### run 967 (Mar 13) — no spec changes, all stable
- Spec scan: no new consensus-specs commits since #5001 (Mar 12). No new release or spec-test vectors.
- **PR #4939**: rebased on master (fdfad73e31), no semantic changes. Still OPEN.
- **PR #4992 (PTC lookbehind)**: unchanged, 1 APPROVED (jtraglia), MERGEABLE. Still OPEN.
- **PR #4940**: test file rename only. Still OPEN.
- All 9 tracked PRs unchanged. CI green. Nightly green (3 consecutive). Docker build in progress. cargo audit unchanged (1 rsa).
- Reviewed test coverage across state_processing/gloas.rs (~208 tests), fork_choice (~290 tests), beacon_chain/tests/gloas.rs (~290 tests). Coverage is comprehensive — no significant gaps found.

### runs 960-965 consolidated (Mar 13) — all stable, no code changes
Spec completely stable — no new consensus-specs commits since #5001 (Mar 12). All tracked PRs remained OPEN.
- Run 964: PR #4939 got wording update ("Review feedback"), no semantic changes — our implementation still matches.
- Run 963: Rebased `ptc-lookbehind` branch onto main. Clean rebase, 575/575 tests pass.
- Run 961: PR #5002 got APPROVED (jtraglia). No code impact.
- Run 960: Consolidated progress log (445→140 lines).
- CI and nightly continuously green. cargo audit unchanged (1 rsa, 5 warnings).

### runs 925-959 consolidated (Mar 13) — PR #5001 implemented, PTC lookbehind approaching merge
Spec stable — no new consensus-specs commits with consensus changes since #5001 (Mar 12), no new release, no new spec-test vectors. All tracked PRs remained OPEN.

Notable activities:
- Run 929: Implemented PTC lookbehind on branch `ptc-lookbehind` (previous_ptc/current_ptc fields, compute_ptc, get_ptc cached reads, per_slot rotation, upgrade initialization). All 575 state_processing tests pass. NOT merged — blocks on PR #4992 merge.
- Run 929: Fixed `clippy::large_stack_frames` in `proposer_boost_re_org_test` (Rust 1.91 bump)
- Run 927: Merged community PR #30 (Rust 1.88→1.91, Bullseye→Trixie Docker base)
- Run 926: Updated deps (clap 4.6, openssl 0.10.76, c-kzg 2.1.7, tempfile 3.27)
- Run 925: Implemented PR #5001 (`parent_block_root` bid filtering key). PR #4979 CLOSED; #4992 surviving approach.
- PR #4992 evolved from 2 commits (head 215962a9, blocked) to 8 commits (head d76a278b0a, mergeable=clean, 1 APPROVED jtraglia Mar 12).

### runs 746-897 consolidated (Mar 10) — race fix, dep updates, spec stable
Spec completely stable. All 11 tracked Gloas PRs remained OPEN. CI and nightly continuously green.

Key code changes:
- Run 829: Fixed remaining race in `data_column_reconstruction_at_deadline` (now drains all gossip events before breaking)
- Run 826: Initial fix for flaky nightly test (race condition in event ordering)
- Run 847: Updated schannel 0.1.28→0.1.29
- Run 820: Committed Cargo.lock update (enr syn 1→syn 2)
- Run 810: Reviewed PR #4992 diff — implementation plan ready
- Run 804: Updated libz-sys 1.1.24→1.1.25

### runs 642-745 consolidated (Mar 9) — spec conformance fixes, test improvements
Spec stable. All 10-11 tracked Gloas PRs remained OPEN. CI and nightly continuously green.

Key activities:
- Run 735: Fixed 2 beacon_chain test failures (slasher backend guard, Fulu fork scheduling check) — 766→768/768
- Run 723: Added 15 proto_array propagation tests
- Run 720: Fixed spec conformance bug in `process_execution_payload_envelope` (self-build signature skip)
- Run 719: Added PTC lookbehind bug demonstration test
- Run 718: Deep spec conformance audit — all Gloas functions verified correct
- Run 701: Implemented index-1 attestation envelope validation (PR #4939) proactively
- Run 671: Added 3 edge case tests for bid overwrite behavior

### runs 608-641 consolidated (Mar 8-9) — rebranding, devnet verification
- Runs 611-619: Full vibehouse rebranding (binary, crates, docs, CLI, metrics, P2P agent)
- Run 640: Post-rebrand devnet verification SUCCESS (finalized_epoch=8)
- Run 641: Docker CI paths-ignore for docs-only commits

### 2026-03-08 — initial audit found all changes already implemented
- Compared consensus-specs master against v1.7.0-alpha.2 tag
- 4 Gloas spec files changed: beacon-chain.md, fork-choice.md, p2p-interface.md, validator.md
- All consensus-critical changes were already in vibehouse (implementing from spec PRs ahead of the release tag)
