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
| #5008 | Fix field name `block_root` → `beacon_block_root` in EnvelopesByRoot spec prose | Doc-only; our code already uses `beacon_block_root` correctly |
| #5022 | Add check that block is known in `on_payload_attestation_message` | Already implemented (UnknownBeaconBlockRoot error at fork_choice.rs:1426-1432) |
| #5023 | Fix block root filenames and Gloas comptests | Test infra only; no code change needed |
| #5015 | Integrate make coverage into make test | Test infra only; no code change needed |

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

### run 1863 (Mar 18) — routine check

- No new Gloas spec commits since Mar 13 (#5002)
- #4992 (cached PTCs in state) still open, updated Mar 17 — will implement when merged

### run 1885 (Mar 18) — deep spec audit + full verification

- Deep audit of fork choice implementation against latest consensus-specs master
  - `process_execution_payload_bid`: all validation steps match spec exactly (self-build, builder active, balance check, sig verify, blob limit, slot, parent_block_hash, parent_block_root, prev_randao, pending payments, bid caching) ✓
  - `is_supporting_vote_gloas`: PENDING/EMPTY/FULL logic matches spec; `message.slot == block.slot → false` correct (validated by `validate_on_attestation` ensuring `slot >= block_slot`) ✓
  - `get_gloas_weight`: returns 0 for non-PENDING previous-slot nodes ✓
  - `should_extend_payload`: checks both `ptc_weight > threshold` AND `ptc_blob_data_available_weight > threshold` ✓
  - `get_payload_tiebreaker`: EMPTY=1, FULL=2 (if should_extend) or 0, non-previous-slot=ordinal ✓
  - `validate_on_attestation`: index in [0,1], same-slot must be 0, index-1 requires payload_revealed ✓
  - `on_execution_payload`: marks payload_revealed + envelope_received + payload_data_available ✓
  - Bid pool: filters by parent_block_root, highest bid tracking uses (slot, parent_block_hash, parent_block_root) ✓
  - `PayloadAttestationData`: has both `payload_present` and `blob_data_available` fields ✓
- EF spec tests: 139/139 pass (fake_crypto+minimal), 9/9 fork choice tests pass (real crypto)
- Fork choice unit tests: 324/324 pass
- No new merged Gloas PRs since last check
- Open Gloas PRs unchanged: #4992, #4898, #4892, #4843, #4840, #4747, #4630, #4558, #5008, #4939, #4954
- All other tracked open PRs still unmerged
- CI green (all workflows passing), `cargo check` clean (no warnings)
- Pinned to v1.7.0-alpha.3 (latest release)
- Scanned heze fork specs for awareness: introduces FOCIL (EIP-7805, fork-choice enforced inclusion lists) — 16-member inclusion list committee, new `InclusionList` type, fork choice integration. Early stage, no action needed yet.

### run 1886 (Mar 18) — routine check

- No new Gloas spec commits since Mar 15 (#5005), no new release since alpha.3
- #4992 (cached PTCs in state) still open, under active discussion (potuz, jihoonsong, ensi321)
- Open Gloas PRs unchanged: #4992, #5008, #4962, #4960, #4939, #4932, #4843, #4840, #4630
- CI green: all workflows passing (ci, nightly, spec-test-version-check)
- Nightly flakes (Mar 16 slasher, Mar 17 range sync) both already fixed; Mar 18 nightlies green
- `cargo check` clean, `cargo audit`: same known advisory (RUSTSEC-2023-0071 rsa/jsonwebtoken), no new issues
- **No action needed. Will re-check when new PRs merge or alpha.4 is released.**

### run 1899 (Mar 18) — routine spec check + devnet verification

- No new Gloas spec commits since Mar 15 (#5005), no new spec-test release since alpha.3
- Open Gloas PRs unchanged: #4992, #5008, #4962, #4960, #4939, #4932, #4843, #4840, #4630, #4898, #4892, #4954
- #4940 (initial fork choice tests) merged Mar 13 — adds `test_on_execution_payload` test + `head_payload_status` check format. Our EF test runner already supports `head_payload_status` checks. Ready for next spec-test release.
- CI green, clippy clean (zero warnings), all EF tests passing
- Devnet test passed: finalized_epoch=8, chain healthy through Gloas fork (4 nodes, minimal preset)
- **No action needed. Will re-check when new PRs merge or alpha.4 is released.**

### run 1905 (Mar 18) — routine health check

- No new consensus-specs merges since #5005 (Mar 15), no new release since alpha.3
- Open Gloas PRs unchanged: #4992 (cached PTCs, active discussion), #5008 (field name fix), #4954 (milliseconds), #4898, #4892, #4843, #4840, #4630
- All 9 fork choice EF test categories pass including on_execution_payload from #4940
- CI green (ci + nightly), zero clippy warnings, zero doc warnings, zero compiler warnings
- cargo audit: same known advisory (RUSTSEC-2023-0071 rsa/jsonwebtoken, no fix available), no new issues
- jsonwebtoken 10.3.0 is latest — no upgrade path for rsa vulnerability
- All codebase TODOs audited: all blocked (EIP-7892, blst upstream, PeerDAS) or non-critical
- **No action needed. Codebase healthy, spec current.**

### runs 1923-2057 (Mar 19-21) — routine spec checks (consolidated)

- No new consensus-specs merges since #5005 (Mar 15), no new release since alpha.3
- Monitored open Gloas PRs throughout:
  - **PTC lookbehind**: 3 competing approaches (#4979 full 2-epoch cache 256KB, #4992 per-slot cache, #5020 minimal 4KB) — design unsettled
  - **#4843** (variable PTC deadline): significant design change, still under discussion
  - **#5022** (block-known check in on_payload_attestation_message): already handled via `UnknownBeaconBlockRoot` at fork_choice.rs:1432
  - **#5008** (field name fix `block_root` → `beacon_block_root`): doc-only, our code already correct
  - **#5023** (block root filenames + Gloas comptests): test infrastructure only
- Verified codebase health across 30+ runs: CI green (ci + nightly), zero clippy/compiler warnings, EF tests 139/139 pass, cargo audit unchanged (RUSTSEC-2023-0071 rsa, unfixable)
- Code quality verified: zero production `.unwrap()` in consensus/, all unsafe blocks legitimate (5 total: libc FFI, blst crypto, env var), all wildcard imports intentional
- **Will implement PTC lookbehind or variable PTC deadline when merged.**

### run 2128 (Mar 21) — routine spec check + dep update verification

- No new consensus-specs merges since #5005 (Mar 15), no new release since alpha.3
- Open Gloas PRs unchanged: PTC lookbehind (#4979), variable PTC deadline (#4843), block-known check (#5022), test infra (#5023)
- CI running on latest dep updates (console-subscriber 0.5, igd-next 0.17, rusqlite 0.39, r2d2_sqlite 0.33): check+clippy+fmt ✓, ef-tests ✓, others in progress
- Nightlies: 5/5 green (Mar 17-21)
- `cargo check` clean (zero warnings), `cargo audit` unchanged (RUSTSEC-2023-0071 rsa, unfixable; new unmaintained: bincode RUSTSEC-2025-0141, not actionable)
- Remaining outdated deps: rand_xorshift 0.4→0.5, rand 0.8→0.9 (breaking, requires workspace-wide rand migration — deferred)
- **No action needed. Spec current, codebase healthy.**

### run 2129 (Mar 21) — routine check, CI verified green

- No new consensus-specs merges since #5005 (Mar 15), no new release since alpha.3
- Open Gloas PRs unchanged: PTC lookbehind (#4979/#4992/#5020), variable PTC deadline (#4843), block-known check (#5022 — already implemented), test infra (#5023), fork choice milliseconds (#4954), remove pending from tiebreaker (#4898), remove impossible branch (#4892)
- CI fully green on latest dep updates (run 23382597558): all 6 jobs passed including beacon_chain, http_api, unit tests
- Ran `cargo machete --with-metadata`: no actionable unused deps (all flagged are false positives from `TestRandom` derive macro needing `rand` in scope)
- Ran `cargo clippy --release --all-targets`: zero warnings
- Ran dead code check (`RUSTFLAGS="-W dead_code"`): zero warnings on core crates

### run 2243 (Mar 23) — PTC lookbehind resolution + spec tracking

- **PTC lookbehind settled**: #4992 and #5020 both CLOSED in favor of #4979 (full 2-epoch + lookahead cache). #4979 is the only surviving approach:
  - Adds `ptc_lookbehind: Vector[Vector[ValidatorIndex, PTC_SIZE], (2+MIN_SEED_LOOKAHEAD)*SLOTS_PER_EPOCH]` to BeaconState (~256KB)
  - `compute_ptc(state, slot)` — pure computation (extracted from current `get_ptc`)
  - `get_ptc(state, slot)` — becomes cache lookup into `ptc_lookbehind`
  - `process_ptc_lookbehind(state)` — new epoch processing: shift window + fill lookahead
  - `initialize_ptc_lookbehind(state)` — fork upgrade: empty previous epoch + compute current epoch
  - Implementation impact: BeaconState SSZ change, epoch processing addition, fork upgrade logic, split get_ptc into compute+cache
  - **Status**: still OPEN, not merged. Will implement when merged.
- **Recently merged**: #5008, #5022, #5027 — all already tracked and confirmed aligned
- No new consensus-specs release since alpha.3
- Duplicate deps in Cargo.lock: all transitive (strum 0.27 from sp1 stack, rand versions from various ecosystems)
- **No action needed. Spec current, codebase healthy.**

### run 2215 (Mar 22) — spec audit + codebase health

- 3 new Gloas spec commits since last check:
  - **#5001** (parent_block_root in bid filtering key): already implemented — `is_highest_value_bid` uses `(Slot, ExecutionBlockHash, Hash256)` 3-tuple
  - **#5002** (wording clarification for self-build signature verification): doc-only, no code change
  - **#5008** (field name fix `block_root` → `beacon_block_root` in EnvelopesByRoot): doc-only, our code already correct
- Approved PRs ready to merge: #5022 (block-known check — already implemented), #4898 (remove pending tiebreaker — already aligned), #4892 (remove impossible branch — already aligned)
- Nightly failure (Mar 22): transient infrastructure issue — `cargo-nextest@0.9.132` binary wasn't uploaded to GitHub when nightly ran; binary available now, next nightly will pass
- CI (push): check+clippy+fmt ✓, ef-tests ✓, remaining jobs in progress
- `cargo clippy --release --all-targets`: zero warnings
- `RUSTFLAGS="-W dead_code" cargo check --release`: zero warnings
- Production `.expect()` audit: all 64 occurrences in state_processing are in `#[cfg(test)]` blocks — zero production panics
- **No action needed. Spec current, codebase healthy.**

### run 2236 (Mar 23) — spec audit + CI health + deep conformance verification

- **#5014** merged (Mar 22): EIP-8025 P2P protocol update — removes Metadata/GetMetaData changes, adds `ExecutionProofStatus` and `ExecutionProofsByRange` RPCs. Not yet implemented (our ZK proofs use gossip subnets + HTTP API only; P2P sync RPCs deferred until real SP1 devnet).
- Open Gloas PRs reviewed: #4892, #4898, #5022 all approved/clean — vibehouse already aligned with all three
- PTC caching bug (#4992): understood the epoch-boundary PTC divergence. Spec-level bug, fix not merged yet; will implement when it lands.
- **Deep fork choice conformance**: verified `is_supporting_vote_gloas_at_slot` uses `==` (not `<=`) with assertion comment; `get_payload_tiebreaker` omits PENDING check (equivalent because `collect_gloas_children` never places PENDING in tiebreaker position); `get_head` comparison order (weight, root, tiebreaker) matches spec exactly.
- Local tests: fork choice 327/327, state_processing 1026/1026, proto_array all pass
- Nightly failure (Mar 23): `MEGABYTE` dead code in slasher redb-only build — already fixed in commit 5d23ecf85; nightly failure (Mar 22): transient nextest 404 (not our issue)
- CI: check+clippy+fmt ✓, ef-tests ✓, network+op_pool ✓, remaining jobs in progress
- **No action needed. Spec current, codebase healthy.**

### run 2262 (Mar 23) — spec audit, 4 new merged PRs

- **4 new Gloas-related PRs merged** since last spec audit:
  - **#5022** (block-known check in `on_payload_attestation_message`): already implemented — `UnknownBeaconBlockRoot` error at fork_choice.rs:1426-1432
  - **#5014** (EIP-8025 P2P protocol update): removes MetaData v4, adds `ExecutionProofStatus` and `ExecutionProofsByRange` RPCs. Not yet implemented — our ZK proof infrastructure uses gossip+HTTP; P2P sync RPCs deferred until real SP1 devnet
  - **#5008** (field name fix `block_root` → `beacon_block_root` in EnvelopesByRoot spec prose): doc-only, our code already uses `beacon_block_root`
  - **#5023** (fix block root filenames + Gloas comptests): test infrastructure only — changes how fixtures name block files (BeaconBlock root, not SignedBeaconBlock root), adds `head_payload_status` to `output_store_checks`, and adds `on_execution_payload` step support to compliance runner. Our EF test runner already handles all three: `head_payload_status` checks, `on_execution_payload` steps, and `execution_payload_envelope_*.ssz_snappy` file loading. No action needed until new fixtures are generated.
- PTC window (#4979): still OPEN, actively discussed (updated today). Will implement when merged.
- Nightly failures (Mar 22, 23): both resolved — nextest 404 (transient), MEGABYTE dead code (fixed in 5d23ecf85)
- CI green, zero clippy warnings, zero compiler warnings
- **No action needed. Spec current, codebase healthy.**

### run 2264 (Mar 23) — spec compliance verification + PTC window update

- Verified compliance with all recently merged spec PRs via deep code audit:
  - **#5022** (block-known check): fork choice validates at line 1426-1432, gossip validates at gloas_verification.rs check 3 — both return explicit errors for unknown blocks ✓
  - **#5008** (field name): `ExecutionPayloadEnvelope` struct correctly uses `beacon_block_root` field name throughout ✓
- **PTC window (#4979)**: rename discussion today — `ptc_lookbehind` → `ptc_window` agreed upon (terencechain, jtraglia, Mar 23). Design converging but not merged.
- Local test verification after dependency update: types 1085/1085, state_processing 1026/1026, fork_choice 327/327 — all pass
- CI: check+clippy+fmt ✓, ef-tests ✓, remaining jobs in progress
- Clean build, zero clippy warnings, zero compiler warnings, cargo fmt clean
- cargo audit: same known advisory (RUSTSEC-2023-0071 rsa, unfixable), no new issues
- **No action needed. Spec current, codebase healthy.**

### run 2266 (Mar 23) — routine spec check + CI verification

- No new functional Gloas spec merges — latest commits are CI/tooling only (#5029 setup-uv, #5030 release-drafter, #5028 download-artifact, #5031 workflow merge, #5023 block root filenames)
- **#5023** (block root filenames + Gloas comptests): merged today. Verified our EF test runner is resilient — uses YAML manifest references (not hash-based filename discovery), so block file naming changes won't break loading when new fixtures are released
- **PTC window (#4979)**: still OPEN, 12 review comments, rename to `ptc_window` agreed. No approvals yet — still in design discussion
- Nightly failures (Mar 22, 23): both explained — Mar 22 was transient nextest 404, Mar 23 was MEGABYTE dead code (already fixed in 5d23ecf85, will be picked up by tonight's nightly)
- CI fully green: all 6 jobs passed (run 23460108181)
- Codebase audit: zero actionable TODOs (all blocked on external factors), zero production unwrap()/expect() outside tests, zero clippy warnings
- cargo audit: same known advisories (RUSTSEC-2023-0071 rsa unfixable, plus unmaintained transitive deps: bincode, ansi_term, derivative, filesystem, paste — all not actionable)
- **No action needed. Spec current, codebase healthy.**

### run 2267 (Mar 23) — routine spec check

- No new functional Gloas spec merges since run 2266 — only CI/tooling PRs (#5029, #5030, #5028, #5031)
- **PTC window (#4979)**: still OPEN, 0 approvals, rename to `ptc_window` agreed. Design converging but not merged
- Open Gloas PRs unchanged: #4979 (PTC window), #4954 (milliseconds), #4843 (variable PTC deadline), #4898 (1 approval), #4892 (2 approvals), #4939, #4840, #4747, #4630, #4558
- Nightly slasher failure (Mar 22-23): already fixed in 5d23ecf85 (cfg guard on MEGABYTE). Tonight's nightly should pass
- CI green (run 23460108181), zero clippy warnings, cargo audit unchanged
- Attempted rand_xorshift 0.4→0.5 upgrade: blocked by rand_core version split (0.6/0.9/0.10 — three versions in tree). Deferred
- **No action needed. Spec current, codebase healthy.**

### run 2270 (Mar 23) — routine spec check

- No new functional Gloas spec merges — only CI/tooling commits (#5027, #5029, #5030, #5031, deps)
- **PTC window (#4979)**: still OPEN, not merged. Design converging (rename to `ptc_window` agreed)
- Nightly slasher failure (Mar 23 09:33 UTC): confirmed caused by MEGABYTE dead code in redb-only build. Fix was pushed at 10:42 UTC (commit 5d23ecf85) — after the nightly ran. Tonight's nightly should pass.
- CI green (push workflows), zero clippy warnings, zero dead code warnings, zero compiler warnings
- Rust stable 1.94.0 (current), only outdated dep is rand_xorshift 0.4→0.5 (blocked by rand_core split)
- **No action needed. Spec current, codebase healthy.**

### run 2271 (Mar 23) — deep spec + toolchain audit

- Full re-audit of all 6 post-alpha.3 merged PRs: all confirmed implemented or not-applicable
  - **#5001** (parent_block_root bid filtering): verified — `observed_execution_bids.rs` uses 3-tuple key `(Slot, ExecutionBlockHash, Hash256)` ✓
  - **#5022** (block-known check): verified — 2 layers: gossip check + fork choice proto-array lookup ✓
  - **#5008** (field name fix): doc-only ✓
  - **#5023** (test fixtures): no impact until new test release ✓
  - **#5014** (EIP-8025 P2P): deferred (ZK infra uses gossip+HTTP) ✓
  - **#5002** (wording): no code change ✓
- **PTC window (#4979)**: still OPEN, mergeable_state=blocked, 12 review comments, 0 approvals
- Latest EF test release: v1.7.0-alpha.3 (Mar 13) — we're on it
- Latest consensus-specs release: v1.7.0-alpha.3 — we're on it
- Nightly clippy (Rust 1.96.0-nightly): zero warnings
- Stable clippy (Rust 1.94.0): zero warnings
- cargo audit: 1 vuln (RUSTSEC-2023-0071 rsa, no fix available), 5 unmaintained (transitive, not actionable)
- All 3 remaining TODOs linked to #36, all blocked on external factors
- **No action needed. Spec current, codebase healthy.**

### run 2279 (Mar 24) — routine spec check + full PR audit

- Scanned all consensus-specs PRs merged to `master` since alpha.3 (Mar 13): 7 relevant PRs found (#5001, #5002, #5005, #5008, #5014, #5022, #5023) — all already tracked in previous runs
- **PTC window (#4979)**: still OPEN, `mergeable_state=blocked`, 12 review comments, 0 approvals. Design stable (rename to `ptc_window` agreed). Full diff reviewed — no changes since last check
- **Variable PTC deadline (#4843)**: still OPEN, 10 review comments, 0 approvals
- No new consensus-specs release since alpha.3, no new EF test fixtures since v1.6.0-beta.0
- CI green (push): all jobs passing (run 23471211833). Nightly (Mar 23) slasher failure = MEGABYTE dead code, fixed in 5d23ecf85, awaiting next nightly run
- cargo audit: same known advisories (RUSTSEC-2023-0071 rsa unfixable, 5 unmaintained transitive deps not actionable). `filesystem` crate warning is false positive (our local crate, not the crates.io one)
- cargo outdated: only rand_xorshift 0.4→0.5 (blocked by rand_core version split)
- **No action needed. Spec current, codebase healthy.**

### run 2288 (Mar 24) — spec check + clippy lint expansion

- Spec audit: no new Gloas PRs merged since last check. #4979 (PTC window) still OPEN. #4843 (variable PTC deadline) still OPEN. No new releases or test fixtures.
- CI: latest push (clippy fix, run 23481206829) — check+clippy passed, tests in progress. Nightly slasher failure already fixed (5d23ecf85), awaiting tonight's run.
- **Enforced 42 new clippy lints** (326 → 368): correctness guards (await_holding_lock, uninit_vec, unconditional_recursion, mutable_key_type, out_of_bounds_indexing, wrong_transmute), performance (box_default, drain_collect, unnecessary_sort_by), transmute safety. All pass without code changes.
- cargo audit: same known advisories, no new issues.

### run 2290 (Mar 24) — spec check + full EF test verification

- Spec audit: no new Gloas PRs merged since last check. 3 new merged PRs are all test infrastructure (#5023 merged Mar 23) — already handled by our test runner.
- **PTC window (#4979)**: still OPEN, renamed to `ptc_window`, design stable but 0 approvals. Competing PRs #4992 and #5020 both closed in its favor.
- Open Gloas PRs unchanged: #4979, #4954, #4843, #4898, #4892, #4939, #4960, #4932, #4840, #4747, #4630, #4558
- consensus-specs nightly test generation broken upstream (Mar 8-9 runs cancelled); last successful nightly (Mar 7) is older than alpha.3, so not useful for ahead-of-release testing
- EF tests verified locally: 139/139 (fake_crypto+minimal) + 79/79 (real_crypto+minimal) = 218/218 all passing
- Nightly CI (Mar 24): passed (MEGABYTE fix from 5d23ecf85 landed)
- CI (push, run 23485195611): 4/6 jobs passed (check+clippy, ef-tests, network+op_pool, http_api), unit tests + beacon_chain still running
- Codebase health: zero clippy warnings, zero unwrap()/expect() in consensus production code, all TODOs tracked to #36 (all blocked on external factors)
- **No action needed. Spec current, codebase healthy.**

### run 2310 (Mar 24) — unwrap elimination + spec check

- **Fixed 5 `checked_sub().unwrap()` in validator_client/src/lib.rs**: genesis wait logic (lines 741-776) replaced with `saturating_sub()`. Safe by branch guard but redundant unwraps.
- **Fixed `Withdrawals::new().unwrap()` in execution_layer/src/lib.rs:2200**: replaced with `.and_then(|w| Withdrawals::new(w).ok())` to avoid panic on oversized withdrawal lists.
- No new Gloas spec merges since #5023 (Mar 23). Only CI/infra commits (#5015, #5029, #5030, #5028, #5031).
- **PTC window (#4979)**: still OPEN, not merged. Will implement when merged.
- CI green (full lint passed on push), all tests pass.
- **No action needed. Spec current, codebase healthy.**

### run 2313 (Mar 24) — comprehensive runtime safety audit + full EF test verification

- **Runtime unwrap audit**: comprehensive scan of all runtime (non-test) code in consensus/, beacon_node/, validator_client/. Result: **zero production unwrap()/expect() calls** remaining in consensus/state_processing, consensus/fork_choice, or validator_client service files. All prior cleanup work confirmed effective.
- **EF test verification**: 139/139 (fake_crypto+minimal) + 79/79 (real_crypto+minimal) = 218/218 all passing, including Gloas fork choice `on_execution_payload` tests from #4940.
- **Spec tracking**: no new Gloas spec merges since #5023 (Mar 23). PTC window (#4979) still OPEN (mergeable_state=blocked, 0 approvals, 12 review comments, last updated Mar 23).
- **CI**: latest push (run 23496276572) completed successfully. Zero clippy warnings.
- **No action needed. Spec current, codebase healthy.**
