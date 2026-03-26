# Fast Confirmation Rule (FCR) Implementation Design

**Spec PR**: ethereum/consensus-specs#4747
**Research paper**: https://arxiv.org/abs/2405.00549
**Status**: Open (not merged). This doc is pre-implementation research.

## What FCR Does

FCR identifies blocks that are "safe from reorgs" under honest-majority and synchrony assumptions. It replaces the current safe block definition (`justified_checkpoint.root`) with a dynamically computed `confirmed_root` that tracks 1-2 slots behind the head, rather than 1-2 epochs.

The `confirmed_root` becomes the Engine API `safeBlockHash` via `forkchoiceUpdated`.

## New Store Fields (6)

| Field | Type | Purpose |
|-------|------|---------|
| `confirmed_root` | `Root` | Most recent confirmed block root |
| `previous_epoch_observed_justified_checkpoint` | `Checkpoint` | Justified checkpoint observed by all honest nodes at start of previous epoch |
| `current_epoch_observed_justified_checkpoint` | `Checkpoint` | Justified checkpoint observed by all honest nodes at start of current epoch |
| `previous_epoch_greatest_unrealized_checkpoint` | `Checkpoint` | Greatest unrealized justified checkpoint at end of previous epoch |
| `previous_slot_head` | `Root` | Head root at start of previous slot |
| `current_slot_head` | `Root` | Head root at start of current slot |

All initialized from anchor state: `confirmed_root = anchor_root`, checkpoints = `justified_checkpoint`, heads = `anchor_root`.

## New Config Constants

| Name | Value | Description |
|------|-------|-------------|
| `CONFIRMATION_BYZANTINE_THRESHOLD` | `25` | Max assumed Byzantine validator percentage |
| `COMMITTEE_WEIGHT_ESTIMATION_ADJUSTMENT_FACTOR` | `5` | Per-mille adjustment for partial-epoch committee estimates |

## Core Algorithm

`on_fast_confirmation(store)` runs once per slot, after past attestations are applied and before the attestation deadline:

1. **`update_fast_confirmation_variables`**: Rotate slot heads, snapshot checkpoints at epoch boundaries
2. **`get_latest_confirmed`**: The main algorithm:
   - **Revert**: If confirmed_root is >1 epoch old, not on canonical chain, or reconfirmation fails → revert to `finalized_checkpoint.root`
   - **Restart**: At epoch start, if observed justified checkpoint is ahead of confirmed_root → restart from justified root
   - **Advance**: Walk canonical chain from confirmed_root toward head, checking `is_one_confirmed` for each block
3. **`is_one_confirmed(store, balance_source, block_root)`**: Does the block have enough attestation weight that no competing block could outweigh it, even with 25% adversary?
   - `support = get_attestation_score(store, block_root, balance_source)`
   - `safety_threshold = (max_support + proposer_score + 2*adversarial - discount) / 2`
   - Returns `support > safety_threshold`

## ~25 New Functions

### Misc helpers
- `get_block_slot`, `get_block_epoch`, `get_checkpoint_for_block`, `get_current_target`, `is_start_slot_at_epoch`, `is_ancestor`, `get_ancestor_roots`

### State helpers
- `get_slot_committee` (needs committee shuffling for current_epoch - 2)
- `get_pulled_up_head_state` (process slots on head state — potentially expensive)
- `get_previous_balance_source`, `get_current_balance_source` (checkpoint states for observed justified)

### LMD-GHOST safety
- `get_block_support_between_slots`, `is_full_validator_set_covered`, `adjust_committee_weight_estimate_to_ensure_safety`
- `estimate_committee_weight_between_slots` (pro-rata epoch boundary math)
- `get_equivocation_score`, `compute_adversarial_weight`, `get_adversarial_weight`
- `compute_empty_slot_support_discount`, `get_support_discount`
- `compute_safety_threshold`, `is_one_confirmed`

### FFG safety
- `get_current_target_score`, `compute_honest_ffg_support_for_current_target`
- `will_no_conflicting_checkpoint_be_justified`, `will_current_target_be_justified`

### Core
- `update_fast_confirmation_variables`, `find_latest_confirmed_descendant`, `get_latest_confirmed`, `on_fast_confirmation`

## Vibehouse-Specific Implementation Notes

### Where the Store lives
- **Trait**: `ForkChoiceStore<E>` in `consensus/fork_choice/src/fork_choice_store.rs`
- **Impl**: `BeaconForkChoiceStore` in `beacon_node/beacon_chain/src/beacon_fork_choice_store.rs`
- Add 6 new fields to `BeaconForkChoiceStore`, expose via trait methods

### Initialization
- `BeaconForkChoiceStore::get_forkchoice_store()` (beacon_fork_choice_store.rs:153-202)
- Initialize all 6 fields from anchor state/checkpoint

### Safe block semantics change
- Currently: no explicit `get_safe_execution_block_hash()` — justified checkpoint is used
- After FCR: `confirmed_root`'s execution block hash becomes `safeBlockHash`
- In `recompute_head_at_slot_internal()` (canonical_head.rs:635-754), the `ForkchoiceUpdateParameters` caching needs to use `confirmed_root` instead of justified checkpoint for safe hash

### Per-slot integration
- `on_fast_confirmation` must be called once per slot at precise timing
- Current flow: `per_slot_task()` → `recompute_head_at_current_slot()` → `get_head()`
- FCR hook goes after `get_head()` but before attestation deadline
- Add `on_fast_confirmation()` call in `recompute_head_at_slot_internal()` after `get_head()` completes

### Gloas compatibility challenges
1. **`get_head` returns `ForkChoiceNode(root, payload_status)` in Gloas**, not just `Root`. FCR calls `get_head` in ~5 places expecting a Root. Need to extract `.root` from the node.
2. **`LatestMessage` uses `slot` not `epoch` in Gloas**. The spec provides `get_latest_message_epoch()` to bridge this.
3. **`get_attestation_score` takes `ForkChoiceNode` in Gloas** instead of `Root`. FCR functions need to construct `ForkChoiceNode` or we need a Root-accepting variant.

### Performance concerns
- `get_head()` is called multiple times internally by FCR (in `get_current_target`, `get_slot_committee`, etc.). Each is a full proto-array recomputation. **Must cache head result.**
- `get_slot_committee` needs committee shuffling for `current_epoch - 2`. May need to cache committee data or use existing committee cache infrastructure.
- `get_pulled_up_head_state` processes slots on head state — cache or avoid repeated calls.

### Implementation phases

**Phase 1: Store fields & initialization**
- Add 6 fields to `ForkChoiceStore` trait and `BeaconForkChoiceStore`
- Initialize in `get_forkchoice_store()`
- Add to serialization/persistence if applicable

**Phase 2: Helper functions**
- Implement misc helpers (ancestry, block slot/epoch lookups)
- Implement LMD-GHOST safety functions
- Implement FFG safety functions

**Phase 3: Core algorithm**
- `update_fast_confirmation_variables`
- `find_latest_confirmed_descendant`
- `get_latest_confirmed`
- `on_fast_confirmation`

**Phase 4: Integration**
- Wire `on_fast_confirmation` into per-slot processing
- Update `ForkchoiceUpdateParameters` to use `confirmed_root` for safe hash
- Handle Gloas `ForkChoiceNode` compatibility

**Phase 5: Testing**
- Unit tests for each helper function
- Integration tests for full FCR flow
- EF spec tests (when released — PR has 9 test files, ~6000 lines)
- Devnet verification

## Estimated Effort

Medium-High. ~25 new functions, precise slot timing integration, Gloas compatibility layer, performance optimization (head caching, committee lookups). The individual functions are mostly straightforward math, but the integration surface area is large.

## Dependencies

- PR #4747 must be merged before implementation starts (currently still in review)
- May need committee cache improvements for lookback to `current_epoch - 2`
- EF test vectors needed for validation (9 test categories defined in the PR)
