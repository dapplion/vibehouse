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

### run 787 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still blocked (needs approvals)
- CI green, docker workflow still queued (self-hosted runner availability)
- No code changes needed

### run 786 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- Recent merges since last scan: none (last was #4995, Python 3.14, CI only)
- PTC lookbehind #4992: unchanged, still blocked (needs approvals)
- CI green, cargo audit unchanged (rsa advisory only), no compatible dep updates
- EF spec tests: running (background)
- No code changes needed

### run 785 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still blocked (needs approvals), design stable
- CI green, cargo audit unchanged (rsa advisory only)
- No actionable TODOs in Gloas production code (only pre-existing DAS/Electra items)
- No code changes needed

### run 784 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still open
- CI green, cargo audit unchanged (rsa advisory only)
- No code changes needed

### run 783 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still open
- CI green, nightly green (3 consecutive), cargo audit unchanged (rsa advisory only)
- Docker workflow still queued since Mar 9 (self-hosted runner availability)
- No code changes needed

### run 782 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still blocked (needs approvals)
- CI green, zero compiler warnings, cargo audit unchanged
- Docker workflow still queued since Mar 9 (self-hosted runner availability)
- No code changes needed

### run 781 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still open
- Spec test vectors: still v1.5.0, no new release
- Codebase: zero compiler warnings, clean clippy, CI green
- TODO audit: 23 pre-existing TODOs (DAS feature work, pool persistence, design notes) — none Gloas-actionable
- No code changes needed

### run 780 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still open
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- No code changes needed

### run 778 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still open
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- No code changes needed

### run 777 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still open
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- No code changes needed

### run 776 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- No new consensus-relevant merges since #4995 (Python 3.14, CI only)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still open
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- No code changes needed

### run 775 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- No new consensus-relevant merges since #4995 (Python 3.14, CI only)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still open
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- CI green; no code changes needed

### run 774 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- Recent merges (#4993-4995): CI/tooling only, no consensus changes
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still blocked (needs approvals)
- Clippy clean, zero warnings; cargo audit unchanged (rsa advisory only)
- Docker workflow queued ~13h (self-hosted runner, not code)
- No code changes needed

### run 773 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC, still open
- EF spec tests: 35/35 passed; CI green; nightly green (3 consecutive)
- No compatible dependency updates; no code changes needed

### run 772 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- Recently merged #4995 (Python 3.14) — CI only, no consensus changes
- PTC lookbehind #4992: unchanged since Mar 10 01:07 UTC (potuz responding to review, design converging)
- EF spec tests: 35/35 passed; CI green; zero compiler warnings
- Code quality audit: all TODOs are pre-existing design notes, no actionable bugs, no unwraps in consensus paths
- No code changes needed

### run 771 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since Mar 9 (ensi321 prefers it over #4979, potuz addressing feedback)
- Docker workflow queued ~12h (self-hosted runner unavailability)
- No code changes needed

### run 770 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since last scan (last activity Mar 9), still open
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- Compilation clean, zero warnings
- No code changes needed

### run 769 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: unchanged since last scan, still open
- Codebase audit: zero compiler warnings, zero clippy warnings, comprehensive Gloas test coverage confirmed (can_builder_cover_bid 10+ tests, envelope_processing 57 tests, gloas_verification 51 tests), no missing Gloas fork handling in match statements
- Docker workflow queued >4h (self-hosted runner availability, not code)
- No code changes needed — next real work: PTC lookbehind implementation when #4992 merges

### run 768 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- Recent merges (#4995): Python 3.14 support — CI/tooling only, no consensus changes
- PTC lookbehind #4992: unchanged since potuz's Mar 10 01:07 UTC updates (removed `get_ptc_assignment`, addressed all review feedback). Mergeable but blocked (needs approvals).
- EF spec tests: 35/35 passed; CI green; cargo audit unchanged (rsa advisory only)
- No code changes needed

### run 767 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: potuz pushed fresh updates (5 new comments within last few hours) — removed `get_ptc_assignment` helper entirely, addressed review feedback. Design: `ptc_lookbehind: Vector[Vector[ValidatorIndex, PTC_SIZE], 2]`, rotated each slot in `process_slots`, `get_ptc` becomes pure lookup with slot assertion. Fork choice reorders `on_payload_attestation_message` to check slot before calling `get_ptc`. Validator `get_ptc_assignment` removed.
- EF spec tests: 35/35 passed; CI green
- No code changes needed — will implement when PR merges

### run 766 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- Recent consensus-specs merges: python 3.14 support (#4995), reftest/action cleanup — all CI/tooling only
- PTC lookbehind #4992: unchanged (10 review comments, design still evolving)
- EF spec tests: 35/35 passed; CI green (22875542079); cargo check: zero warnings
- No code changes needed

### run 765 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- No new PRs after #4995; PTC lookbehind #4992 unchanged since last scan
- EF spec tests: 35/35 passed; fork choice tests: 8/8 passed; CI green; nightly green (Mar 7-9)
- Cargo audit unchanged (rsa advisory only); no compatible dependency updates available
- Production code audit: no actionable TODOs in consensus/fork_choice paths

### run 764 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992: last updated ~7h ago, no structural changes; design still evolving
- EF spec tests: 35/35 passed; CI green; nightly green (3 consecutive: Mar 7-9); cargo audit unchanged (rsa advisory only)
- No code changes needed

### run 763 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- Recent merges: EIP-6800 (Verkle) specs removed (#4984), rest CI/tooling/cleanup — no consensus changes
- PTC lookbehind #4992: updated today but no structural changes; design still evolving
- EF spec tests: 35/35 passed; CI green; nightly green (3 consecutive); cargo audit unchanged (rsa advisory only)
- Code quality audit: zero unwraps in Gloas production code, zero TODOs in Gloas paths, all #[allow] attrs benign
- No code changes needed

### run 762 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- New merge #4995 (Python 3.14 support) — CI/tooling only, no consensus changes
- PTC lookbehind #4992 unchanged since Mar 9 (ensi321 comment preferring over #4979)
- EF spec tests: 35/35 passed; clippy clean; CI green; cargo audit unchanged (rsa advisory only)
- No compatible dependency updates; no code changes needed

### run 761 (Mar 10)
- Spec scan: all tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind #4992 unchanged since Mar 9; no new consensus-relevant merges
- EF spec tests: 35/35 passed; cargo check clean; CI green
- No code changes needed

### run 758 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- No new activity on PTC lookbehind PRs (#4992/#4979) since Mar 9
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset, fake_crypto)
- Clippy clean; cargo audit unchanged (rsa advisory only); no compatible dependency updates
- CI green; nightly green (Mar 7-9, Mar 10 not yet scheduled)
- No code changes needed

### run 757 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind PR #4992: no new activity since Mar 9, design still evolving
- Full EF spec tests: 138/138 passed (fake_crypto, minimal preset, 0 skipped)
- Fork choice EF tests: 8/8 passed (real crypto, minimal preset — get_head, on_block, ex_ante, reorg, withholding, deposit_with_reorg, get_proposer_head, should_override_forkchoice_update)
- Clippy clean on consensus crates; no compatible dependency updates; CI green; nightly green

### run 756 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- Recent merges (#4990-4994): all CI/tooling/cleanup, no consensus changes
- PTC lookbehind PR #4992: no new activity since Mar 9, design still evolving
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset)
- Clippy clean on consensus crates

### run 755 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- PTC lookbehind PR #4992: ensi321 comment preferring #4992 over #4979; 5 review comments from potuz/kevaundray; design still evolving
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset)
- Clippy clean on consensus + beacon_chain crates; no compatible dependency updates; cargo audit unchanged (rsa advisory only)
- CI green; production code unwrap audit: only `dump_as_dot()` diagnostic utility has unwraps, all consensus paths safe

### run 754 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset)
- Beacon chain tests: 768/768 passed (FORK_NAME=gloas, full suite)
- Clippy clean on consensus crates; no compatible dependency updates; cargo audit unchanged (rsa advisory only)
- CI green: 5 nightly green runs, CI green, spec-test-check green
- Block production code audit: no TODOs, no issues in Gloas paths (bid selection, payload attestation packing, self-build envelope, EL fetch)

### run 753 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2)
- Devnet verification SUCCESS: finalized_epoch=8, clean Gloas fork (run ID 20260310-010545)
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset)
- Clippy clean on consensus + beacon_chain crates; no compatible dependency updates
- Code review: envelope_processing, per_slot_processing (payload availability), process_withdrawals_gloas — all correct

### run 752 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), spec-test vectors still v1.5.0
- PTC lookbehind PR #4992: new comment from ensi321 preferring #4992 over #4979; potuz acknowledged duties functions not yet addressed; design still evolving
- CI green; docker workflow still queued (self-hosted runner availability, not code); clippy clean, zero warnings
- No compatible dependency updates available (30 major-version bumps only)
- Codebase audit: no actionable TODOs in Gloas production paths, all Gloas beacon_chain functions have integration test coverage
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset)

### run 751 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new PRs since #4994
- PTC lookbehind PR #4992: no new activity since Mar 9, design still evolving
- CI green; docker workflow still queued (runner availability); clippy clean on all consensus crates
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset)
- Cargo audit unchanged (rsa advisory only, no fix available)

### run 750 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0 for mainline, custom for Gloas)
- PTC lookbehind PR #4992: no new activity since Mar 9, design still evolving
- Recently merged consensus-specs PRs (#4990, #4991, #4993, #4994): all CI/tooling/cleanup, no consensus changes
- CI green; docker workflow still queued (runner availability issue); nightly: 5 consecutive green runs (Mar 5-9)
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset)
- Cargo audit unchanged (rsa advisory only, no fix available)

### run 749 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.5.0)
- PTC lookbehind PR #4992: no new activity since Mar 9, design still evolving
- CI green; docker workflow queued >24h (likely runner issue, not code)
- Workspace tests: 2643/2643 passed (excluded web3signer_tests — external service)
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset)
- Clippy clean, cargo audit unchanged (rsa advisory only, no fix available)

### run 748 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.5.0)
- PTC lookbehind PR #4992: no new comments since last scan (last activity Mar 9), design still evolving
- Recently merged consensus-specs PRs (#4990-4994): all CI/tooling/cleanup, no consensus changes
- CI fully green; nightly tests: 7 consecutive green runs (Mar 5-10)
- Workspace tests: 2643/2643 passed (excluded web3signer_tests — external service)
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset)
- Cargo audit unchanged (rsa advisory only, no fix available)
- Code audit: no TODO/FIXME/unreachable!/todo! in Gloas production code (beacon_chain)

### run 747 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.5.0)
- PTC lookbehind PR #4992: no new comments since last scan (last activity Mar 9), design still evolving
- CI fully green; nightly tests: 6 consecutive green runs (Mar 6-10)
- Workspace tests: 2643/2643 passed (excluded web3signer_tests — external service)
- EF spec tests: 35/35 passed (operations + epoch_processing + sanity, minimal preset)
- Clippy clean, cargo audit unchanged (rsa advisory only)

### run 746 (Mar 10)
- Spec scan: all 11 tracked PRs still OPEN, no new Gloas merges, no new spec release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.5.0)
- PTC lookbehind PR #4992: no new comments since last scan, design still evolving
- CI fully green: all 7 jobs passed; nightly tests: 5 consecutive green runs (Mar 6-10)
- Workspace tests: 2643/2647 passed (4 web3signer_tests timeout — external service, not code)

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
