# Blinded Payload Envelopes (Issue #8888)

## Objective
Prevent unbounded disk growth by splitting execution payload envelopes into a compact blinded form (retained permanently) and a large full payload (pruned on finalization).

## Status: DONE

### Problem
In Gloas (ePBS), every execution payload envelope stored in the `BeaconEnvelope` database column contained the full execution payload (transactions, withdrawals). These are large (~1 MB per block) and were never pruned, causing unbounded disk growth even after finalization.

### Solution
Introduced `SignedBlindedExecutionPayloadEnvelope` — an envelope containing the execution payload header (roots) instead of the full payload body. On store:
- Blinded envelope → `BeaconEnvelope` column (small, kept forever)
- Full Gloas payload → `ExecPayload` column (large, pruned on finalization)

On retrieval, the full envelope is reconstructed by combining blinded + payload. After pruning, block replay uses the blinded envelope with state-derived withdrawals (transactions are not needed during replay).

### Files Changed
- `consensus/types/src/blinded_execution_payload_envelope.rs` — new type with `from_full()`, `into_full()`, `into_full_with_withdrawals()` + 6 tests
- `consensus/types/src/lib.rs` — module + re-export
- `beacon_node/store/src/impls/execution_payload_envelope.rs` — `StoreItem` impl changed to blinded type
- `beacon_node/store/src/lib.rs` — column description updated
- `beacon_node/store/src/hot_cold_store.rs` — split storage, reconstruction, blinded fallbacks in replay/freeze/advanced-state paths
- `beacon_node/store/src/reconstruct.rs` — blinded fallback in state reconstruction
- `consensus/state_processing/src/block_replayer.rs` — `blinded_envelopes` field + three-tier fallback (full → blinded → bid hash)
- `beacon_node/beacon_chain/src/beacon_chain.rs` — `load_envelopes_for_blocks` returns (full, blinded) tuple
- `beacon_node/beacon_chain/src/data_availability_checker/state_lru_cache.rs` — blinded fallback
- `beacon_node/http_api/src/{attestation_performance,block_packing_efficiency,block_rewards}.rs` — updated for tuple return

### Testing
- 6 unit tests for `BlindedExecutionPayloadEnvelope` (SSZ roundtrip, metadata preservation, reconstruction)
- 14 block replayer tests pass (including blinded envelope fallback paths)
- 280 state_processing tests pass
- 24 store tests pass
- 688 types tests pass
- Full workspace clippy lint passes
