# Devnet Testing

## Objective

Test vibehouse under diverse devnet scenarios beyond the happy path. The initial 4-node devnet (priority 1) proved basic functionality — this task covers syncing, node churn, long-running, and adversarial scenarios.

## Status: IN PROGRESS

### Scenarios

| Scenario | Status | Detail |
|----------|--------|--------|
| Syncing (genesis sync) | DONE (script) | `--sync` flag: 2 validators + 2 sync targets, nodes catch up through Gloas fork |
| Node churn | DONE (script) | `--churn` flag: kill validator node 4, verify chain continues (75% stake), restart, verify recovery |
| Mainnet preset | DONE (script) | `--mainnet` flag: 4 nodes, 512 validators, 32 slots/epoch, 12s slots, ~40 min timeout |
| Long-running | DONE (script) | `--long` flag: epoch 50 target, periodic memory/CPU monitoring, ~40 min |
| Builder path | TODO | External bids via API, envelope reveal flow |
| Payload withholding | TODO | Bid without reveal, fork choice handles it |
| Network partitions | TODO | Split nodes, reconnect, test fork resolution |
| Stateless + ZK | DONE | 3 proof-generators + 1 stateless node (from priority 4) |
| Slashing scenarios | TODO | Double-propose / surround-vote, verify detection |

## Progress log

### 2026-02-25 — Genesis sync test (run 108)

**Implemented the syncing devnet test scenario** — the top-priority item from PLAN.md priority 5.

**What was built:**

1. **`kurtosis/vibehouse-sync.yaml`** — Kurtosis config for sync testing:
   - 2 validator nodes (keep running, produce blocks)
   - 2 non-validator nodes (sync targets — stopped and restarted)
   - Same ePBS geth, Gloas fork at epoch 1, minimal preset

2. **`--sync` flag in `kurtosis-run.sh`** — Two-phase sync test:
   - **Phase 1 (finalization):** Start all 4 nodes, immediately stop the 2 non-validators, wait for validators to finalize past epoch 4 (well past the Gloas fork at epoch 1). This creates a chain with both pre-fork (Fulu) and post-fork (Gloas/ePBS) blocks
   - **Phase 2 (sync verification):** Restart EL first (CL needs EL), then CL. Poll both non-validator nodes every 6s (one slot), monitoring:
     - Standard sync API (`/eth/v1/node/syncing` — head_slot, is_syncing)
     - Lighthouse-specific sync state (`/lighthouse/syncing` — SyncingFinalized, SyncingHead, Synced)
   - **Success criteria:** Both nodes report `is_syncing: false` with non-zero head_slot
   - **Post-sync verification:** Queries finality checkpoints and fork version on both sync targets to confirm they're on the Gloas fork with correct finalization

**Key design decisions:**
- Stops both CL and EL for non-validators (not just CL) — ensures the EL also needs to sync, testing the full stack
- Restarts EL before CL — the CL needs the EL to be responsive for engine API calls during sync
- Separate `sync.log` file for the sync phase — easier debugging than mixing with health.log
- 6-minute sync timeout — generous for minimal preset (only ~32 slots to sync at epoch 4)
- `TARGET_FINALIZED_EPOCH=4` (not 8) — enough finalization to prove the chain works, but faster startup for the interesting sync phase

**What this tests:**
- Range sync across the Gloas fork boundary (Fulu blocks → Gloas blocks)
- Block processing pipeline for both pre-fork and post-fork blocks during sync
- State transition through the fork upgrade during catch-up
- ePBS-specific sync behavior (blocks with bids but no inline payload, envelope processing)
- EL sync coordination (engine API during catch-up)

**Usage:**
```bash
scripts/kurtosis-run.sh --sync              # Full test
scripts/kurtosis-run.sh --sync --no-build   # Skip Docker build
scripts/kurtosis-run.sh --sync --no-teardown # Leave running for inspection
```

### 2026-02-26 — Node churn test (run 109)

**Implemented the node churn devnet test scenario** — kill a validator node mid-run, verify the chain continues finalizing, restart it, verify recovery.

**What was built:**

**`--churn` flag in `kurtosis-run.sh`** — Four-phase churn test using the default 4-node config:

- **Phase 1 (warm-up):** Start all 4 validator nodes, wait for finalization to epoch 3 (past Gloas fork at epoch 1). Uses the standard `vibehouse-epbs.yaml` config (no separate config needed).

- **Phase 2 (kill + verify continued finalization):** Stop node 4 (both CL and EL). Wait for finalization to advance at least 2 more epochs with only 3/4 nodes running (75% of stake). This proves the chain handles validator loss gracefully.

- **Phase 3 (restart):** Restart EL first (CL needs EL), then CL — same pattern as sync test.

- **Phase 4 (verify recovery):** Poll the restarted node every 6s until it reports `is_syncing: false` with non-zero `head_slot`. Also monitors the Lighthouse-specific sync state (`/lighthouse/syncing`). After recovery, verifies finality checkpoints on the restarted node.

**Key design decisions:**
- Reuses default `vibehouse-epbs.yaml` config — no separate config file needed since all 4 nodes are identical validators
- `TARGET_FINALIZED_EPOCH=3` for warm-up — enough to prove chain is healthy, faster than the normal target of 8
- `CHURN_FIN_TARGET = PRE_CHURN_FINALIZED + 2` — requires 2 additional finalized epochs with node down, proving sustained chain health
- 3-minute timeout for continued finalization (2 epochs at ~96s each in minimal preset)
- 6-minute timeout for node recovery
- Separate `churn.log` file for all churn phase output
- Kills node 4 specifically (last node) — consistent with sync test pattern

**What this tests:**
- Chain resilience to validator node loss (75% stake threshold for finalization)
- Continued block production and finalization with reduced validator set
- Node recovery after being offline (range sync back to chain head)
- ePBS envelope processing during recovery (blocks with bids during the offline period)
- EL+CL coordination during restart

**Usage:**
```bash
scripts/kurtosis-run.sh --churn              # Full test
scripts/kurtosis-run.sh --churn --no-build   # Skip Docker build
scripts/kurtosis-run.sh --churn --no-teardown # Leave running for inspection
```

### 2026-02-26 — Mainnet preset test (run 110)

**Implemented the mainnet preset devnet test scenario** — run with realistic mainnet parameters instead of the fast minimal preset.

**What was built:**

1. **`kurtosis/vibehouse-mainnet.yaml`** — Kurtosis config for mainnet preset:
   - 4 nodes (vibehouse CL + geth EL), same as default
   - `preset: mainnet` — 32 slots/epoch, 12s/slot, TARGET_COMMITTEE_SIZE=128, PTC_SIZE=512
   - `num_validator_keys_per_node: 128` — 512 total validators
   - Gloas fork at epoch 1 (slot 32)

2. **`--mainnet` flag in `kurtosis-run.sh`** — Overrides timing constants:
   - `SLOTS_PER_EPOCH=32`, recalculates `GLOAS_FORK_SLOT`
   - `POLL_INTERVAL=24` (2 mainnet slots)
   - `TARGET_FINALIZED_EPOCH=4` (past Gloas fork)
   - `TIMEOUT=2400` (40 minutes — mainnet epochs are ~6.4 min each)

**Key design decisions:**
- 512 validators (128/node) — enough for meaningful committee sizes, though smaller than real mainnet. With mainnet TARGET_COMMITTEE_SIZE=128, we get 1 committee per slot (512 / (32 × 128) ≈ 0.125, clamped to 1).
- 40-minute timeout — mainnet finalization is much slower: ~6.4 min/epoch, and we need 4+ epochs to justify + finalize past the Gloas fork.
- No dora (explorer) — reduces resource overhead for the longer-running test.
- Same health polling loop — the script's generic health check (finalization tracking, stall detection) works with any preset.

**What this tests:**
- Mainnet-preset committee sizes and committee assignment logic
- PTC dynamics with PTC_SIZE=512 (vs 2 in minimal)
- Longer epoch times (attestation aggregation over 32 slots)
- Gloas fork transition at realistic timing (slot 32 instead of slot 8)
- General chain health with heavier compute per epoch

**Usage:**
```bash
scripts/kurtosis-run.sh --mainnet              # Full test (~40 min)
scripts/kurtosis-run.sh --mainnet --no-build   # Skip Docker build
scripts/kurtosis-run.sh --mainnet --no-teardown # Leave running for inspection
```

### 2026-02-26 — Long-running test (run 111)

**Implemented the long-running devnet test scenario** — sustained chain health for 50 epochs with periodic resource monitoring.

**What was built:**

**`--long` flag in `kurtosis-run.sh`** — Extended run using the default 4-node config:

- `TARGET_FINALIZED_EPOCH=50` — ~50 epochs × 48s/epoch ≈ 40 min in minimal preset
- `TIMEOUT=3000` (50 min) — generous margin for the long run
- Periodic resource monitoring: every 5th poll (~60s), samples `docker stats` for all CL/EL containers
- Resource snapshots logged to `resources.log` with container name, memory usage, and CPU %
- Memory usage summary printed to stdout alongside chain health

**Key design decisions:**
- Reuses default `vibehouse-epbs.yaml` config — no separate config needed
- 50-epoch target ensures ~40 min of continuous chain operation, well past the "30+ min" goal
- Resource monitoring via `docker stats --no-stream` — non-intrusive, captures memory and CPU per container
- Separate `resources.log` — easy to grep for memory trends over the run duration
- Same stall detection as other modes — if chain stops advancing for 3 consecutive polls, test fails

**What this tests:**
- Memory leak detection over sustained operation (40 min of block production, attestation, finalization)
- Chain stability over many epochs (50 epochs with continuous block production)
- State management under sustained load (state cache behavior over many slots)
- Resource usage trends (growing memory = potential leak)

**Usage:**
```bash
scripts/kurtosis-run.sh --long              # Full test (~40 min)
scripts/kurtosis-run.sh --long --no-build   # Skip Docker build
scripts/kurtosis-run.sh --long --no-teardown # Leave running for inspection
```
