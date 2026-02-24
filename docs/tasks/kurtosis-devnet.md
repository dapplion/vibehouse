# Kurtosis Testing — epbs-devnet-0

## Objective
Participate in epbs-devnet-0 (launch target: Feb 18, 2026). Run vibehouse + geth in kurtosis, verify gloas fork works.

## Status: DONE — 4-node devnet reaches finalized_epoch=8

### Specs
- Consensus specs: `v1.7.0-alpha.2` (we're already on this ✅)
- Only self-built payloads tested (no builder marketplace)
- Fork name: `gloas`, fork epoch: 1, preset: `minimal`
- Reference: https://notes.ethereum.org/@ethpandaops/epbs-devnet-0

### Tasks

#### Step 1: Run reference config (no vibehouse)
- [x] Install kurtosis CLI
- [ ] Run devnet config with consensoor + geth
- [ ] Confirm: chain starts, reaches epoch 1 (gloas fork), finalizes

#### Step 2: Build vibehouse docker image
- [x] Create `Dockerfile.dev` (minimal ubuntu:24.04 + pre-built binary)
- [x] Create `scripts/build-docker.sh` (host cargo build + docker package, ~30s incremental)
- [x] Build local image: `scripts/build-docker.sh` produces `vibehouse:local`
- [x] Verify image runs (must build with `--features spec-minimal` for minimal preset)

#### Step 3: Run kurtosis with vibehouse
- [x] vibehouse CL + geth EL — boots, connects to geth
- [x] Does it produce blocks pre-fork? — YES, slots 0-7 work
- [x] Does it survive gloas fork at epoch 1? — YES
- [x] Does it produce gloas blocks (self-built payloads)? — YES
- [x] Does chain finalize post-fork? — YES, finalized_epoch=8

#### Step 4: Fix issues
- [x] Boot/startup failures — fixed (spec-minimal feature flag)
- [x] Fork transition / block publishing — fixed data column bypass, self-build envelope flow
- [x] Block production 400s — fixed payload bid validation, state caching
- [x] payload_attestation_data 500s — fixed endpoint implementation

#### Step 5: 4-node devnet (DONE)
- [x] Run 4 vibehouse CL + geth EL nodes in kurtosis
- [x] Transactions via spamoor finalize through epoch 8 — finalized_epoch=8, slot 80
- [x] All 4 nodes stay synced and producing blocks
- [x] Chain doesn't stall across multi-node gossip

### Blockers
1. Block production — needs self-build (DONE ✅)
2. Payload envelope import — needs full state transition wiring
3. EL integration — `newPayload` for gloas payloads

### Kurtosis configs
- `kurtosis/vibehouse-epbs.yaml` — 4 vibehouse CL + geth EL (self-test, homogeneous)
- `kurtosis/vibehouse-multiclient.yaml` — 2 vibehouse + 2 lodestar CL + geth EL (interop test)
- `kurtosis/vibehouse-stateless.yaml` — 3 proof-generators + 1 stateless node (ZK proofs)

Key params: gloas_fork_epoch=1, preset=minimal, spamoor + dora enabled.

### Infrastructure
- `Dockerfile.dev` — minimal dev image (ubuntu:24.04 + binary copy, no Rust in Docker)
- `scripts/build-docker.sh` — host `cargo build --release --features spec-minimal` + docker package (~30s incremental)
- `scripts/kurtosis-run.sh` — bounded lifecycle: build → clean → start → poll beacon API → teardown
  - Flags: `--no-build`, `--no-teardown`, `--stateless`, `--multiclient`
  - Polls `/eth/v1/node/syncing` + `/eth/v1/beacon/states/head/finality_checkpoints`
  - Success = finalized epoch >= 8, stall detection (36s), 12-min timeout
  - Multi-client mode: cross-client health check (all 4 nodes compared)
  - Logs to `/tmp/kurtosis-runs/<RUN_ID>/` (build.log, kurtosis.log, health.log, dump/)
- `docs/devnet-checks.md` — full list of 12 health checks for agent debugging

## Progress log

### 2026-02-24: multi-client interop config
- Added `kurtosis/vibehouse-multiclient.yaml`: 2 vibehouse + 2 lodestar CL nodes with geth EL
- Updated `scripts/kurtosis-run.sh`: `--multiclient` flag, cross-client health check, resilient API discovery
- Prep for epbs-devnet-0 (March 4, 2026) — vibehouse needs to interop with lodestar
- All tests pass: 8/8 EF fork choice, 280/280 state_processing, release build clean

### 2026-02-17: deep devnet-0 readiness audit — all clear
- **EF tests**: 136/136 pass, check_all_files_accessed passes (209,677 files, 122,748 excluded)
- **Compilation**: cargo check --release clean, no clippy warnings
- **Block import pipeline**: Gloas blocks correctly bypass execution payload gossip validation, DA checker marks as Available(NoData), bid validations in block_verification.rs correct
- **EL integration**: newPayload correctly called via envelope pipeline, execution_requests sent as 4th param, fork choice marked Optimistic before EL validation (correct per spec)
- **Fork transition**: upgrade_to_gloas properly gated in per_slot_processing, gossip topics subscribe on fork activation
- **Configuration**: gloas_fork_epoch parsed from YAML through full Config→ChainSpec→runtime chain. Kurtosis YAML works.
- **VC integration**: PayloadAttestationService fully implemented and wired, PTC duty endpoints working
- ~~**Spec gap (non-blocking)**: Fork choice missing payload_data_availability_vote~~ — **FIXED**: separate `ptc_blob_data_available_weight` + `payload_data_available` tracking, full `should_extend_payload` implementation
- **Blocker**: Docker still not available

### 2026-02-17: comprehensive devnet-0 readiness audit (clean)
- **Compilation**: `cargo check --release` — clean
- **Clippy**: `cargo clippy --release --workspace -- -D warnings` — zero warnings
- **EF tests**: 136/136 pass, check_all_files_accessed passes
- **Audit scope**: Searched all ePBS code paths for `todo!()`, `unimplemented!()`, `unwrap()`, stale TODOs, hardcoded placeholders
- **Findings**: Codebase is devnet-0 ready. Only finding was a stale TODO comment in gossip_methods.rs (removed).
- **Verified**: VC payload attestation service fully implemented, self-build flow complete, fork choice Gloas model correct
- **Blocker**: Docker still not available — cannot build image or run Kurtosis

### 2026-02-17: spec compliance fixes for deposit routing + bid validation
- **Fix 1**: `process_deposit_request_gloas` routing had extra `is_pending_validator` check not in spec — removed
- **Fix 2**: Execution bid gossip validation had inverted `execution_payment` check (rejected non-zero instead of zero) — fixed
- **Cleanup**: Removed stale TODOs that referred to already-implemented features
- All 136/136 EF tests + check_all_files_accessed pass, clippy clean
- Commit: `0aeabc122`
- **Blocker**: Still need Docker for image build + kurtosis

### 2026-02-17: devnet readiness assessment & clippy cleanup
- **Compilation**: `cargo check --release` passes cleanly
- **EF spec tests**: 78/78 + 136/136 (fake_crypto) all pass
- **Clippy**: Fixed 80+ lint errors across 32 files to pass `cargo clippy --release --workspace -- -D warnings`. Key fixes: safe_arith in consensus code, indexing → `.get()`, collapsed if statements, redundant closures.
- **Dockerfile**: Reviewed — correctly builds lighthouse binary and copies to `/usr/local/bin/lighthouse`
- **Kurtosis configs**: `kurtosis/vibehouse-epbs.yaml` and `kurtosis/epbs-devnet-0.yaml` exist and look correct
- **No runtime blockers**: No `todo!()`, `unimplemented!()`, or `GloasNotImplemented` remain in the codebase
- **Blocker**: Docker not available on current machine — need Docker to build image and run kurtosis
- **Next**: Install Docker, build image, run kurtosis solo devnet

### 2026-02-17: devnet-0 code audit + 3 bug fixes
- **Audit**: Ran comprehensive 3-agent code audit of all ePBS critical paths (self-build flow, VC payload attestation, gossip networking)
- **Bug 1 (CRITICAL)**: `process_self_build_envelope` used stale `cached_head` state — would fail envelope processing after block import. Fixed by fetching post-block state from store.
- **Bug 2 (MINOR)**: `build_self_build_envelope` returned `None` on `Ok(())` path — fixed to return the envelope.
- **Bug 3 (IMPORTANT)**: `verify_payload_attestation_for_gossip` never validated `beacon_block_root` against fork choice — attestations for unknown blocks were accepted. Fixed with fork choice check.
- All 136/136 EF tests + check_all_files_accessed pass
- Clippy clean
- Commit: `cf1078fac`
- **Blocker**: Still need Docker for image build + kurtosis

## Environment
- Docker: installed, `openclaw` user in docker group (use `sg docker "cmd"` for group access)
- Kurtosis: v1.15.2 installed, engine running
- sudo: available, no password required
- **Blocker resolved**: Docker + Kurtosis ready. Build image and launch devnet NOW.

### 2026-02-18: fix gloas block production — use fork choice head_hash not state.latest_block_hash()
- In `get_execution_payload`, for Gloas the `state.latest_block_hash()` can be stale because envelope
  processing runs async and doesn't persist state back to the store.
- Fixed: use `chain.canonical_head.cached_head().forkchoice_update_parameters().head_hash` which is
  authoritative — updated by `on_execution_payload` during envelope processing.
- Added `BlockProductionError::MissingExecutionBlockHash` variant for the `ok_or` error path.
- 136/136 EF tests pass, clippy clean.
- Next: build Docker image, run kurtosis devnet.

### 2026-02-18: devnet infrastructure complete, first run reveals block publishing bug
- **Infrastructure built**:
  - `Dockerfile.dev` — minimal ubuntu:24.04 image, copies pre-built binary
  - `scripts/build-docker.sh` — host cargo build with `--features spec-minimal` + docker package (~30s incremental vs 5-10 min full Docker build)
  - `scripts/kurtosis-run.sh` — full lifecycle with beacon API polling, stall detection, per-run log dirs
  - `docs/devnet-checks.md` — 12 health checks for agent debugging
- **Kurtosis config**: added spamoor for tx load, dropped assertoor (doesn't understand "gloas" fork version)
- **First run result**: chain boots, produces blocks at slots 0-7, then **stalls at slot 7** (gloas fork boundary)
- **Root cause**: Gloas blocks are produced successfully (`/eth/v3/validator/blocks/{slot}` returns 200) but **fail to publish**:
  ```
  ERROR Invalid data column - not publishing data columns  error: PreDeneb, slot: 13
  WARN  Error processing HTTP API request  status: 400 Bad Request, path: /eth/v2/beacon/blocks
  ```
  The block publishing path routes gloas blocks through data column validation which rejects them as "PreDeneb".
- **Secondary issues observed**:
  - `/eth/v1/validator/payload_attestation_data` returns 500 Internal Server Error
  - VC sends duplicate block production requests → "Duplicate payload cached" warnings
  - `/eth/v1/events` returns 400 Bad Request
- **Next**: Fix the block publishing path — gloas blocks should bypass data column publishing (ePBS blocks don't carry data columns in the block, they're in the envelope).

### 2026-02-19: stateless devnet PASSES — finalized_epoch=9

Fixed two critical bugs blocking stateless devnet:
1. **Fork choice stall at skip slots**: `Attestation::empty_for_signing` hardcoded `data.index = 0`
   for all Gloas attestations. Non-same-slot attestations (after skip slots) need `data.index = 1`
   when payload was revealed (per EIP-7732 spec). All votes supported EMPTY, causing fork choice
   traversal to terminate when blocks chain through FULL.
2. **Execution proof import silently discarded**: Gloas blocks bypass the DA checker (imported
   immediately), so `put_execution_proofs` found no entry and dropped proofs. Fixed by tracking
   proofs directly in `execution_proof_tracker` on `BeaconChain`, bypassing DA checker for
   stateless nodes. Also added `pending_execution_proofs` buffer for proof-before-block races.

**Result**: 3 proof-generator CL+EL + 1 stateless CL+EL nodes, gloas fork at epoch 1, spamoor tx
load. Chain reached slot 96, epoch 12, finalized_epoch=9, justified_epoch=11. Some skip slots but
chain recovers. No stalls.

### 2026-02-19: fix non-deterministic StateRootMismatch consensus bug

`get_advanced_hot_state()` in `hot_cold_store.rs` had overrides that replaced the actual state tree
hash root with the caller's `state_root` argument. This wrong root was written into the `state_roots`
array during `per_slot_processing`, causing blocks to fail verification with `StateRootMismatch`
non-deterministically (depending on state advance timer timing). Fix: return actual tree hash root
from `get_advanced_hot_state()`, relax sanity check in `load_parent` for Gloas states.

### 2026-02-18: 4-node devnet PASSES — finalized_epoch=8 achieved

Fixed all remaining issues blocking devnet finalization (commit `6351db47d`):

**Post-envelope state caching fix (critical)**:
- Post-envelope state was cached under envelope's state_root, but block verification and
  state advance use block's state_root to look up state. This caused the pre-envelope state
  (with wrong `latest_block_hash`) to be used for the next block's bid validation, failing
  with "bid parent_block_hash does not match state latest_block_hash".
- Fix: cache post-envelope state under block's state_root using delete+put pattern (since
  `put_state` rejects duplicates, we must delete the pre-envelope entry first).

**Gossip envelope buffering (critical)**:
- Envelopes arriving before their block (common timing race) were permanently dropped because
  `verify_payload_envelope_for_gossip` requires the block root to exist in fork choice.
- Fix: buffer unknown-block envelopes in `pending_gossip_envelopes`, process after block import
  via `process_pending_envelope` called from the block gossip handler.

**Execution validity marking**:
- After EL validates an envelope's payload via newPayload, the block stayed Optimistic because
  recompute_head returned "no change" and never issued forkchoiceUpdated.
- Fix: explicitly call `on_valid_execution_payload` after successful newPayload.

**Other fixes**:
- Load envelope state from store by block.state_root() instead of cached head (race condition)
- Skip BLS signature verification for self-build envelopes (infinity signature)
- Filter payload attestations by parent_block_root for correct block inclusion
- Silently ignore payload attestation slot mismatches per spec
- Add Gloas gossip topics to subscription whitelist
- Don't ban peers for DataColumnsByRange ResourceUnavailable during custody backfill

**Test results**: 52/52 fork_choice, 38/38 state_processing, 17/17 EF operations/sanity, 8/8 EF fork_choice — all pass.

**Devnet result**: 4 vibehouse CL + geth EL nodes, gloas fork at epoch 1, spamoor tx load.
Chain reached slot 80, epoch 10, finalized_epoch=8, justified_epoch=9. No stalls.
