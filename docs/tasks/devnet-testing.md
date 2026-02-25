# Devnet Testing

## Objective

Test vibehouse under diverse devnet scenarios beyond the happy path. The initial 4-node devnet (priority 1) proved basic functionality — this task covers syncing, node churn, long-running, and adversarial scenarios.

## Status: IN PROGRESS

### Scenarios

| Scenario | Status | Detail |
|----------|--------|--------|
| Syncing (genesis sync) | DONE (script) | `--sync` flag: 2 validators + 2 sync targets, nodes catch up through Gloas fork |
| Node churn | TODO | Kill/restart validator nodes mid-run, test recovery |
| Mainnet preset | TODO | Realistic committee sizes, PTC dynamics |
| Long-running | TODO | 30+ min, catch memory leaks and stalls |
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
