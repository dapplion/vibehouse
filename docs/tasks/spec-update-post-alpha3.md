# Spec Update: Post v1.7.0-alpha.3

## Objective
Track and implement consensus-specs changes merged to master since v1.7.0-alpha.3.

## Status: DONE

All Gloas spec PRs merged since alpha.3 have been audited and implemented (or confirmed not needed).

## Changes Audit (run 1748-1749)

Audited all Gloas spec commits since alpha.3 (17 PRs total). All implemented or confirmed not applicable.

### Already Aligned (no code changes needed)

| PR | Description | Status |
|----|-------------|--------|
| #4948 | Reorder payload status constants (Empty=0, Full=1, Pending=2) | Already correct |
| #4869 | Reduce MIN_BUILDER_WITHDRAWABILITY_DELAY (mainnet 64, minimal 2) | Already correct |
| #4884 | Split ptc_vote into payload_timeliness_vote + payload_data_availability_vote | Already implemented |
| #4875 | Move blob_kzg_commitments from envelope to bid | Already implemented (bid has field, envelope doesn't) |
| #4817 | Onboard builders at fork (onboard_builders_from_pending_deposits) | Already implemented |
| #4897 | Check if pending deposit exists before applying to builder (is_pending_validator) | Already implemented |
| #4868 | Onboard builders using pending deposit slot (add_builder_to_registry takes slot) | Already implemented |
| #4918 | Only allow attestations for known payload statuses | Already implemented (payload_revealed check) |
| #4923 | Ignore beacon block if parent payload unknown | Already implemented (GloasParentPayloadUnknown) |
| #5001 | Add parent_block_root to bid filtering key | Already implemented (3-tuple dedup key) |
| #4879 | Allow multiple preferences from validators (per-slot dedup) | Already implemented |
| #4916 | Refactor builder deposit conditions in process_deposit_request | Already implemented |
| #5002 | Make wording clearer for payload signature verification | Doc-only, no code change |
| #4890 | Clarify when builders become active | Doc-only, no code change |
| #4947 | Pre-fork subscription for proposer_preferences topic | Already implemented (PRE_FORK_SUBSCRIBE_EPOCHS=1, ProposerPreferences in Gloas topics) |
| #5005 | Fix builder voluntary exit success test (yield missing fixture) | Test-only; our EF test runner already handles missing fixtures with SkippedKnownFailure |
| #4940 | Add initial fork choice tests for Gloas (genesis + on_execution_payload) | Test-only; our EF test runner already supports on_execution_payload steps, all tests pass |

### Implemented

| PR | Description | Status |
|----|-------------|--------|
| #4874 | Simplify data column sidecar gossip checks in Gloas | DONE (run 1748) |

### No Code Change Needed

| PR | Description | Rationale |
|----|-------------|-----------|
| #4880 | Deferred validation scoring | Queueing implemented. Retroactive downscoring explicitly deferred by spec — gossipsub protocol doesn't support it yet |
| #4950 | Extend by_root serve range to MIN_EPOCHS_FOR_BLOCK_REQUESTS | Already compliant — our by_root handlers serve everything in storage without range restrictions, which is more permissive than the spec minimum |

### Detail: #4874 — Gloas data column sidecar gossip simplification

**Problem**: Current `validate_data_column_sidecar_for_gossip` runs all Fulu checks for Gloas sidecars, but the Gloas spec removes many checks and replaces them with bid-based validation.

**Spec (Gloas p2p-interface.md)**:
For Gloas sidecars (where `bid = block.body.signed_execution_payload_bid.message`):
1. IGNORE: Valid block for sidecar's slot has been seen (queue for deferred validation if not)
2. REJECT: Sidecar's slot matches block's slot
3. REJECT: `verify_data_column_sidecar(sidecar, bid.blob_kzg_commitments)` — structural check
4. REJECT: Correct subnet
5. REJECT: `verify_data_column_sidecar_kzg_proofs(sidecar, bid.blob_kzg_commitments)` — KZG proof check
6. IGNORE: First sidecar for `(beacon_block_root, index)` with valid proof

**Removed from Fulu**:
- Future slot check
- Finalized slot check
- Proposer signature verification
- Parent block check
- Slot-higher-than-parent check
- Finalized descendant check
- Inclusion proof verification
- Proposer index verification

**Implementation plan**:
- Branch `validate_data_column_sidecar_for_gossip` by fork
- For Gloas: look up block/bid by `beacon_block_root`, get `blob_kzg_commitments` from bid
- Pass external commitments to `verify_data_column_sidecar` and `verify_kzg_for_data_column`
- Skip all header/parent/inclusion proof checks

### Detail: #4950 — Extend by_root serve range

**What**: `BeaconBlocksByRoot` and `ExecutionPayloadEnvelopesByRoot` serve range extends from "since latest finalized epoch" to `MIN_EPOCHS_FOR_BLOCK_REQUESTS` epochs back. `BlobSidecarsByRoot` removes the `finalized_epoch` floor.

**Impact**: Low — affects RPC request handler range checks.

## Progress Log

### run 1748 (Mar 17) — spec audit + #4874 implementation

- Audited all 15 functional Gloas spec PRs merged since alpha.3
- 14/15 already implemented in vibehouse
- **Implemented #4874**: Gloas data column sidecar gossip simplification
  - Split `validate_data_column_sidecar_for_gossip` into Fulu and Gloas paths
  - Gloas path: bid-based validation (block lookup → get commitments from bid → structural + KZG verify)
  - Removed Fulu-only checks for Gloas: proposer sig, parent block, inclusion proof, future/finalized slot
  - Added `BlockUnknown` and `SlotMismatch` error variants
  - Added `is_gloas()` method to `DataColumnSidecar`
  - All 201 network tests pass, 414 Gloas beacon_chain tests pass, clippy clean
- 2 lower priority items remain: deferred validation scoring (#4880), by_root serve range (#4950)

### run 1749 (Mar 17) — final audit, close task

- Re-audited all merged Gloas spec PRs — found 2 additional: #4947 (pre-fork subscription), #5005 (test fixture fix)
- #4947: Already covered — `PRE_FORK_SUBSCRIBE_EPOCHS=1` subscribes to all Gloas topics (including ProposerPreferences) 1 epoch before fork
- #5005: Test-only fix — our EF test runner already handles the missing fixture via `SkippedKnownFailure`
- #4880: Retroactive downscoring explicitly deferred by the spec itself — gossipsub doesn't support it. Queueing path works.
- #4950: Our by_root handlers serve everything in storage — already more permissive than spec minimum. No restriction needed.
- **All spec tracking items resolved. Task DONE.**

### run 1750 (Mar 17) — open PR scan

Scanned open PRs in ethereum/consensus-specs for upcoming changes that could affect vibehouse:

**Fork choice (vibehouse already aligned with proposed changes):**
- #4892: Remove impossible branch in `is_supporting_vote` — vibehouse already uses `==` check
- #4898: Remove PENDING from tiebreaker condition — vibehouse already omits PENDING check

**Open design questions (no action yet):**
- #4899: Should proposer boost be counted in `is_parent_strong`? — unresolved
- #4843: Variable PTC deadline — could change PTC timing assumptions
- #4992: Cached PTCs in state — new BeaconState field, under active debate (design not settled)
- #4954: Store.time → Store.time_ms — limited impact (fork choice test handler only)
- #4747: Fast Confirmation Rule — large scope, still under review
- #4630: EIP-7688 forward compatible SSZ — design phase
- #4558: Cell Dissemination via Partial Message — early stage
- #5014: EIP-8025 p2p protocol (ExecutionProofStatus/ExecutionProofsByRange RPCs) — early stage

**New EIPs being bundled into Gloas:**
- #4840: EIP-7843 (SLOTNUM opcode) — EL-side, no CL impact expected

**New test PRs (not merged yet):**
- #4960: Fork choice test with new validator deposit via envelope + reorg
- #4932: Sanity/blocks tests with payload attestation coverage
- #4962: Missed payload + withdrawal interaction tests
- Verified vibehouse handles edge cases from all three test PRs

### run 1773 (Mar 17) — implement envelope request from index-1 attestations

Implemented the SHOULD behavior from the Gloas p2p spec (aligned with open PR #4939):
- When an index-1 attestation arrives but the execution payload envelope hasn't been seen, we now request it via `ExecutionPayloadEnvelopesByRoot` RPC
- Added `SyncMessage::MissingEnvelopeFromAttestation` with debounce (30s per block_root)
- Added `SyncRequestId::SingleEnvelope` for response routing
- Added `Work::RpcPayloadEnvelope` beacon processor work item
- Response processing: verify envelope → process state transition → update fork choice
- All 201 network tests, 61 Gloas beacon_chain tests, 9 EF fork choice test categories pass

### runs 1794-1827 (Mar 17-18) — spec tracking maintenance

- No new consensus-specs merges since #5005 (Mar 15)
- All 11+ tracked open Gloas PRs still open/unmerged
- #4992 (cached PTCs in state) now APPROVED — will implement when merged
  - Adds `previous_ptc`/`current_ptc` to BeaconState, rotates every slot in `process_slots`
  - `get_ptc(state, slot)` reads from cache instead of computing; `compute_ptc(state)` helper added
  - Fork upgrade initializes `current_ptc = compute_ptc(state)`, `previous_ptc = zeros`
- #5008 (field name fix: `block_root` → `beacon_block_root` in EnvelopesByRoot) — doc-only, our code already correct
- Repeatedly verified: CI green, clippy clean, all EF tests pass (139/139 + 79/79), workspace tests 4914/4914 pass
- Nightly flakes from Mar 16-17 already fixed: range test (#8f8faa7), slasher test (#2848be8)
- `cargo audit`: 1 unfixable advisory (RUSTSEC-2023-0071 in `rsa` via `jsonwebtoken`)
- **Will re-check when alpha.4 is released or new PRs merge.**
