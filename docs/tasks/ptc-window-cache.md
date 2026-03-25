# PTC window cache (consensus-specs #4979)

**Status:** DONE
**Spec PR:** https://github.com/ethereum/consensus-specs/pull/4979
**Commit:** 191e3e4bd

## Summary

Proactive implementation of the PTC (Payload Timeliness Committee) window cache in BeaconState. This caches PTC assignments for previous, current, and next+lookahead epochs, avoiding recomputation on every access and fixing the lookbehind bug where balance changes during epoch processing would alter previously-computed committees.

## Changes

### Types (`consensus/types/`)
- `EthSpec::PtcWindowSlots` — type-level constant: `(2 + MIN_SEED_LOOKAHEAD) * SLOTS_PER_EPOCH`
  - Mainnet: 96, Minimal: 24, Gnosis: 48
- `BeaconStateGloas::ptc_window` — `FixedVector<FixedVector<u64, PtcSize>, PtcWindowSlots>`

### State processing (`consensus/state_processing/`)
- `compute_ptc(state, slot, spec)` — core computation (renamed from old `get_ptc_committee`)
- `get_ptc_committee(state, slot, spec)` — reads from `ptc_window` cache for Gloas states, falls back to `compute_ptc`
- `initialize_ptc_window(state, spec)` — creates initial window during fork upgrade (zeros for previous epoch, computed for current + lookahead)
- `process_ptc_window(state, spec)` — shifts window left by one epoch during epoch processing, fills new last epoch
- `SinglePassConfig::ptc_window` — controls whether `process_ptc_window` runs

### Fork upgrade (`upgrade/gloas.rs`)
- Builds committee caches (Current + Next) before initialization
- Calls `initialize_ptc_window` and assigns to state

### EF tests (`testing/ef_tests/`)
- `PtcWindow` epoch processing handler registered
- Typenum consistency check for `PtcWindowSlots`

## Known limitations

- EF Gloas test vectors will fail SSZ deserialization until new vectors including `ptc_window` are released
- `process_ptc_window` fills zeros for the furthest lookahead epoch when committee caches don't cover it (epoch N+2 from state at N); in production, committee caches should be built before calling this function

## Test results

- state_processing: 1033/1033 pass (7 new ptc_window tests added in run 2360)
- types: 1085/1085 pass
- EF tests: Gloas-specific tests fail (expected — SSZ schema mismatch with pre-#4979 test vectors)

## Spec alignment audit (run 2360, 2026-03-25)

Compared vibehouse implementation against latest #4979 spec diff (commit 89ce53b). All behavioral aspects aligned:
- Window size calculation: identical
- Shift logic: identical
- Fork upgrade initialization: correctly handles via upgrade_to_gloas (builds committee caches first)
- Cache lookup (get_ptc): index calculation matches spec
- Genesis path: covered via upgrade chain (genesis.rs calls upgrade_to_gloas which initializes ptc_window)

Unit tests added (commit 3bce060a9):
- `initialize_ptc_window_correct_size` — verifies window has (2+MIN_SEED_LOOKAHEAD)*SPE slots
- `initialize_ptc_window_previous_epoch_zeroed` — first epoch all zeros
- `initialize_ptc_window_current_epoch_populated` — current epoch has non-zero entries
- `get_ptc_committee_reads_from_cache` — cache matches direct compute_ptc
- `get_ptc_committee_previous_epoch_returns_zeros` — previous epoch returns zeros
- `process_ptc_window_shifts_epochs` — verifies left-shift preserves entries correctly
- `process_ptc_window_fills_new_last_epoch` — verifies window length preserved after shift
