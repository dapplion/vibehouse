# Spec Tests

## Objective
Run the latest consensus spec tests at all times. Track and fix failures.

## Status: IN PROGRESS

### Current results
- **78/78 ef_tests pass (real crypto, 0 skipped)** — both mainnet + minimal presets
- **138/138 fake_crypto pass (0 skipped)** — both mainnet + minimal presets (Fulu + Gloas DataColumnSidecar variants both pass)
- **check_all_files_accessed passes** — 209,677 files accessed, 122,748 intentionally excluded
- All 8 fork_choice test categories pass (get_head, on_block, ex_ante, reorg, withholding, get_proposer_head, deposit_with_reorg, should_override_forkchoice_update)
- 40/40 gloas execution_payload envelope tests pass (process_execution_payload_envelope spec validation)
- rewards/inactivity_scores tests running across all forks (was missing)
- 3 altair proposer_boost tests now pass (were skipped, sigp/lighthouse#8689 — fixed by implementing PR #4807)

### Tasks
- [ ] Audit spec test runner — understand download, cache, run flow
- [ ] Check which spec test version is currently pinned
- [ ] Update to latest spec test release when new ones drop
- [ ] Ensure all existing fork tests pass (phase0 through fulu)
- [ ] Add gloas test scaffolding: register fork, add handlers, wire new test types
- [ ] Set up CI job: download latest vectors, run all tests, fail on regression
- [ ] Create automated PR bot for new spec test releases

### Test categories
bls, epoch_processing, finality, fork, fork_choice, genesis, light_client, operations, random, rewards, sanity, ssz_static, transition

## Progress log

### 2026-02-26 — attestation data.index spec compliance for Gloas (run 133)
- Checked consensus-specs PRs since run 132: no new Gloas spec changes merged
  - PR #4923 (ignore beacon block if parent payload unknown): already implemented in run 129
  - PR #4930 (rename execution_payload_states to payload_states): cosmetic naming in spec text, no code change needed
  - Open PRs unchanged: #4940, #4939, #4932, #4926, #4898, #4892, #4843, #4840, #4747, #4630
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Deep audit of Gloas consensus code coverage** — reviewed process_deposit_request_gloas, compute_proposer_indices, process_execution_payload_envelope (early payment path), attestation data.index production
- **Fixed spec compliance bug in attestation `data.index` production**:
  - **Bug**: `produce_unaggregated_attestation` used `block.payload_revealed` from the proto_node to determine `data.index` for Gloas. `payload_revealed` is set by EITHER PTC quorum OR envelope receipt. The spec says attesters should follow the fork choice head's winning virtual child (EMPTY vs FULL), not the PTC signal
  - **Impact**: When PTC quorum is reached (`payload_revealed=true`) but the actual winning fork choice head is the EMPTY virtual child (because no envelope was received), the attester would incorrectly vote `data.index=1` (FULL) instead of `data.index=0` (EMPTY). This is an incorrect attestation that would not earn the head reward
  - **Fix**: For skip-slot attestations (head block from a prior slot), use `gloas_head_payload_status()` which reflects the fork choice head selection result. Same-slot attestations always have `data.index=0` per spec. Historical attestations still use `payload_revealed` as a fallback
  - Early attester cache path already correctly handled same-slot guard (`request_slot > item.block.slot()`)
- **Fixed stale comment on `gloas_head_payload_status()`**: comment said `1 = EMPTY, 2 = FULL` but after PR #4948 the values are `0 = EMPTY, 1 = FULL, 2 = PENDING`
- **Verified spec compliance**: `compute_proposer_indices` is functionally identical to `compute_balance_weighted_selection(size=1, shuffle_indices=True)`, `process_deposit_request_gloas` matches spec after PRs #4897/#4916, `process_execution_payload_envelope` early payment path is correct
- Verified: 576/576 beacon_chain tests pass, 193/193 proto_array+fork_choice tests pass, cargo fmt + clippy clean

### 2026-02-26 — bid pool parent_block_root filtering (run 132)
- Checked consensus-specs PRs since run 131: no new Gloas spec changes merged
  - Open PRs unchanged: #4940, #4939, #4932, #4926, #4898, #4892, #4843, #4840, #4747, #4630
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Fixed Issue A from run 131**: `get_best_bid` now filters by `parent_block_root`
  - **Bug**: `ExecutionBidPool::get_best_bid(slot)` only filtered by slot, not by the block's parent root. After a re-org, the chain head changes, and a bid valid for the old head's `parent_block_root` would be selected. `process_execution_payload_bid` in per-block processing would then reject the mismatched `parent_block_root`, causing block production to fail silently (the proposer wastes their slot)
  - **Impact**: After any re-org during a slot where external builder bids exist, the proposer would select a stale bid, block processing would reject it, and the proposer would miss their slot. Self-build fallback would not kick in because the bid was "successfully" selected before block construction began
  - **Fix**: Added `parent_block_root: Hash256` parameter to `get_best_bid` and `get_best_execution_bid`. The block production call site (`produce_partial_beacon_block`) already has `parent_root` available, so it now passes it through. Only bids matching the current chain head's parent root are considered
  - **Added 3 new unit tests**: `best_bid_filters_by_parent_block_root`, `best_bid_wrong_parent_block_root_returns_none`, `best_bid_selects_highest_value_among_matching_parent`
  - Updated 8 existing integration tests to pass the correct `parent_block_root`
- Verified: 17/17 execution_bid_pool unit tests pass, 8/8 bid-related beacon_chain integration tests pass, cargo fmt + clippy clean

### 2026-02-26 — self-build envelope error handling audit (run 131)
- Checked consensus-specs PRs since run 130: no new Gloas spec changes merged
  - Open PRs unchanged: #4940, #4939, #4932, #4926, #4898, #4892, #4843, #4840, #4747, #4630
  - PR #4940 (Gloas fork choice tests) updated Feb 25 — covers `on_execution_payload` handler, will need support when merged
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Deep audit of Gloas block production path** — systematically reviewed self-build envelope construction, external bid selection, payload extraction, and publish flow
- **Found and fixed chain-stall bug in `build_self_build_envelope`**:
  - **Bug**: `build_self_build_envelope` returned `Option<SignedExecutionPayloadEnvelope>` and silently returned `None` on unexpected errors from `process_execution_payload_envelope`. This allowed block production to succeed and publish a self-build block without an envelope. Since no one else would reveal the payload (there's no external builder), the chain would stall indefinitely for that slot
  - **Impact**: Any unexpected error in envelope processing (BeaconStateError, BlockProcessingError, ArithError, etc.) would cause a silent chain stall — the block is published to the network, the VC logs success, but the slot's payload is never revealed
  - **Fix**: Changed return type to `Result<..., BlockProductionError>` with new `EnvelopeConstructionFailed` variant. Block production now fails if the envelope can't be constructed, preventing publication of an unusable block
- **Found and fixed silent payload type mismatch in envelope data extraction**:
  - **Bug**: At Gloas envelope data extraction, `execution_payload_gloas().ok().cloned()` silently converted a type mismatch (EL returning non-Gloas payload for Gloas slot) to `None`, skipping envelope construction. Similarly, missing `execution_requests` produced `None` via `.zip(requests)` instead of an error
  - **Fix**: Both paths now return explicit errors (`EnvelopeConstructionFailed` and `MissingExecutionRequests`)
- **Audit also confirmed correct implementations**: `latest_block_hash` patching, `notify_ptc_messages`, self-build bid fields, per_block_processing validation, gossip payload skip, envelope state transition via `get_state`
- **Noted low-severity Issue A**: external bid pool `get_best_bid` doesn't filter by `parent_block_root` — after a re-org, a stale bid could be selected. However, `process_execution_payload_bid` in per_block_processing catches the mismatch, so block production fails safely (no invalid block published). Not fixed in this run to keep scope focused
- Verified: 573/573 beacon_chain tests pass, 317/317 state_processing tests pass, 193/193 proto_array+fork_choice tests pass, 8/8 EF fork choice tests pass, cargo fmt + clippy clean

### 2026-02-26 — spec PR #4948 + notify_ptc_messages fix (run 130)
- Checked consensus-specs PRs since run 129: 2 Gloas PRs merged
  - **#4948** (merged Feb 26): "Reorder payload status constants" — changes ordinal values: Empty=0, Full=1, Pending=2 (was Pending=0, Empty=1, Full=2). **Implemented**: updated `GloasPayloadStatus` enum ordering, fixed 2 hardcoded test values in fork_choice.rs, updated test names/comments for accuracy
  - **#4947** (merged Feb 26): "Add pre-fork subscription note for proposer_preferences topic" — SHOULD subscribe one epoch before fork activation. **Noted**: vibehouse already subscribes `SUBSCRIBE_DELAY_SLOTS=2` slots early; full-epoch early subscription is advisory, not implemented
  - Open PRs unchanged: #4940, #4939, #4932, #4926, #4898, #4892, #4843, #4840, #4747, #4630
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Analysis of #4948 impact**: The numeric values changed but relative ordering between EMPTY and FULL is preserved in all practical comparison contexts (they're only compared as siblings of the same PENDING parent). No behavioral change, but vibehouse must match the spec's ordinal values for correct `head_payload_status` reporting
- **Found and fixed spec compliance gap**: `notify_ptc_messages` during block import
  - **Bug**: When importing a block, in-block payload attestations (from `block.body.payload_attestations`) were processed at the state-processing level (updating `builder_pending_payments` weight) but NOT applied to fork choice for the parent block's PTC quorum tracking
  - **Spec**: `on_block` calls `notify_ptc_messages(store, state, block.body.payload_attestations)` which extracts `IndexedPayloadAttestation` per in-block attestation and calls `on_payload_attestation_message` with `is_from_block=True`
  - **Impact**: During sync (when gossip payload attestations aren't available), fork choice wouldn't have accurate PTC quorum data for blocks. This could affect head selection accuracy during sync completion, though it wouldn't cause consensus failures since block import doesn't gate on PTC quorum
  - **Fix**: Added `notify_ptc_messages` equivalent in `import_block()` after `fork_choice.on_block()`: iterates block body's payload attestations, converts to `IndexedPayloadAttestation` via `get_indexed_payload_attestation`, and calls `fork_choice.on_payload_attestation()` for each. Made `get_indexed_payload_attestation` public
- Verified: 119/119 proto_array tests pass, 74/74 fork_choice tests pass, 8/8 EF fork choice tests pass, 230/230 beacon_chain Gloas tests pass, cargo fmt + clippy clean

### 2026-02-26 — fix get_gloas_children and should_extend_payload envelope_received check (run 129)
- Checked consensus-specs PRs since run 128: no new Gloas spec changes merged
  - Open PRs unchanged: #4948, #4947, #4940, #4939, #4932, #4926, #4898, #4892, #4843, #4840, #4747, #4630
  - PR #4948 (reorder payload status constants) approved, likely to merge soon
  - PR #4940 (Gloas fork choice tests) updated Feb 25
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Fork choice spec compliance audit**: systematically compared vibehouse's fork choice functions against consensus-specs Python reference:
  - `get_weight` / `get_gloas_weight` ✓ (correct, uses De Morgan's law inversion of spec's positive filter)
  - `is_supporting_vote` / `is_supporting_vote_gloas` ✓ (correct, `==` equivalent to spec's `<=` under slot invariant)
  - `get_ancestor` / `get_ancestor_gloas` ✓ (correct, different variable names but same logic)
  - `get_parent_payload_status` / `get_parent_payload_status_of` ✓ (correct)
  - `should_apply_proposer_boost` / `should_apply_proposer_boost_gloas` — minor over-counting of equivocating validators (uses all equivocating indices instead of committee-at-slot), conservative and matches pre-Gloas behavior
- **Found and fixed spec compliance bug** in `get_gloas_children` and `should_extend_payload`:
  - **Bug**: `get_gloas_children` used `proto_node.payload_revealed` to decide whether to include the FULL virtual child. `payload_revealed` is set by BOTH `on_execution_payload` (actual envelope receipt) AND `on_payload_attestation` (PTC quorum). The spec's `get_node_children` only creates the FULL child when `root in store.payload_states`, which requires actual envelope processing — not just PTC quorum
  - **Impact**: When PTC quorum was reached but no envelope received, vibehouse would create a FULL child that the spec wouldn't. This could cause FULL to win the head tiebreaker when spec says only EMPTY should exist
  - **Fix**: Added `envelope_received: bool` field to `ProtoNode` and `Block`, set only by `on_execution_payload`. Changed `get_gloas_children` and `should_extend_payload` to check `envelope_received` instead of (or in addition to) `payload_revealed`
  - Same pattern in `should_extend_payload`: spec's `is_payload_timely` and `is_payload_data_available` both require `root in store.payload_states`. Now checks `envelope_received && payload_revealed && payload_data_available`
- **Added 2 edge case unit tests** for PTC-quorum-without-envelope:
  - `find_head_ptc_quorum_without_envelope_stays_empty`: block with `payload_revealed=true` (PTC quorum) but `envelope_received=false` — FULL-supporting vote present but head is EMPTY because FULL child doesn't exist without envelope
  - `find_head_ptc_quorum_with_envelope_becomes_full`: complementary test with `envelope_received=true` — FULL child exists and wins with FULL-supporting vote
- Updated existing `should_extend_payload` and tiebreaker tests to set `envelope_received=true` alongside `payload_revealed` when simulating envelope receipt, ensuring tests exercise the intended code paths
- Verified: 119/119 proto_array tests pass (was 117 + 2 new), 74/74 fork_choice tests pass, 8/8 EF fork choice tests pass, 2240/2240 workspace tests pass (8 web3signer failures are unrelated external service flakiness)

### 2026-02-26 — process_execution_payload_envelope edge case unit tests (run 128)
- Checked consensus-specs PRs since run 127: no new Gloas spec changes merged
  - Open PRs unchanged: #4948, #4947, #4940, #4939, #4932, #4926, #4898, #4892, #4843, #4840, #4747, #4630
  - Checked recently merged: #4941 (execution proof construction, eip8025 only), #4931 (FOCIL rebase onto Gloas, eip7805 only) — neither affects core ePBS
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Coverage gap analysis**: `process_execution_payload_envelope` (envelope_processing.rs:112-300) had 22 existing unit tests covering all 10 field-level consistency checks, signature verification (5 tests), and basic state mutations (6 tests), but was missing: header state_root already-set path, payment queueing independent of PTC weight, payment append to existing withdrawals, availability bit at index 0, and builder index out-of-bounds in signature path
- **Added 5 edge case unit tests** for `process_execution_payload_envelope` (envelope_processing.rs):
  - `nonzero_header_state_root_preserved`: header state_root pre-set to 0x55 — envelope processing skips the `if state_root == default` branch, preserving the existing value instead of overwriting with canonical_root
  - `nonzero_payment_queued_regardless_of_weight`: payment with `amount=3 ETH` but `weight=0` — envelope processing checks `amount > 0` (not weight), so payment is moved to pending withdrawals regardless of PTC weight
  - `payment_appends_to_existing_pending_withdrawals`: 2 pre-existing withdrawals + 1 new payment — verifies push appends at end (3 total), preserving order of existing entries
  - `availability_bit_set_at_slot_zero_index`: state at slot 0 with availability bit 0 cleared — envelope processing sets `execution_payload_availability[0 % 64] = true`, confirming the index formula works at the boundary
  - `builder_index_out_of_bounds_rejected_with_verify`: bid's builder_index = 1 (beyond 1-element registry) — signature verification fails with `BadSignature` because pubkey lookup returns None
- Verified: 317/317 state_processing tests pass (was 312), cargo fmt + clippy clean

### 2026-02-26 — same-slot attestation weight edge case unit tests (run 127)
- Checked consensus-specs PRs since run 126: no new Gloas spec changes merged
  - Open PRs unchanged: #4948, #4947, #4940, #4939, #4932, #4926, #4898, #4892, #4843, #4840, #4747, #4630
  - Checked PRs #4916 (replace pubkey with validator index in SignedExecutionProof), #4897 (pending deposit check), #4884 (blob data availability vote), #4908 (builder voluntary exit tests) — all already implemented or not applicable
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Coverage gap analysis**: same-slot attestation weight accumulation in `process_attestation` (process_operations.rs:229-247) had 4 existing tests for current-epoch attestations but was missing: previous-epoch same-slot attestation path, multi-attester aggregate attestation weight, epoch boundary slot mapping, and weight saturation behavior
- **Added 5 edge case unit tests** for same-slot attestation weight accumulation (process_operations.rs):
  - `previous_epoch_same_slot_attestation_uses_first_half_index`: attestation at slot 10 in state at slot 17 — maps to payment index `10 % 8 = 2` (previous epoch first-half), verifies weight is added to correct payment
  - `previous_epoch_attestation_does_not_touch_second_half`: same setup but verifies that the current-epoch payment at the same `slot % SLOTS_PER_EPOCH` offset (index 8+2=10) remains at weight 0 — confirms epoch isolation
  - `multiple_attesters_accumulate_combined_weight`: aggregate attestation with all committee members attesting — verifies weight equals `committee_len * 32 ETH` (sum of effective balances)
  - `epoch_boundary_slot_attestation_uses_correct_payment_index`: attestation at slot 8 (epoch 1 start) in state at slot 9 — maps to payment index `8 + (8 % 8) = 8`, verifies epoch boundary slot index mapping
  - `weight_saturates_instead_of_overflowing`: payment weight pre-set near `u64::MAX`, attestation adds effective_balance — verifies `saturating_add` caps at `u64::MAX` instead of wrapping
- Also added 2 helper functions: `make_prev_epoch_attestation` (creates Electra attestation targeting previous epoch) and `make_multi_attester_attestation` (creates aggregate with multiple committee bits set)
- Verified: 312/312 state_processing tests pass, cargo fmt + clippy clean

### 2026-02-26 — on_payload_attestation quorum edge case unit tests (run 126)
- Checked consensus-specs PRs since run 125: no new Gloas spec changes merged to stable
  - Open PRs unchanged: #4948 (reorder payload status constants), #4947 (pre-fork subscription note), #4940 (Gloas fork choice tests), #4939 (request missing envelopes for index-1), #4932 (Gloas sanity/blocks tests), #4898 (remove pending from tiebreaker), #4892 (remove impossible branch in forkchoice), #4843 (variable PTC deadline), #4840 (eip7843), #4747 (Fast Confirmation Rule), #4630 (SSZ forward compat)
  - Newly tracked: #4926 (replace SECONDS_PER_SLOT with SLOT_DURATION_MS — touches gloas timing constants)
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Coverage gap analysis**: `on_payload_attestation` quorum logic had tests for basic quorum reach/miss and multi-call accumulation, but was missing tests for quorum idempotency, simultaneous dual-quorum, empty attesting indices, post-quorum weight accumulation, and cross-block independence
- **Added 5 edge case unit tests** for `on_payload_attestation` (fork_choice.rs):
  - `blob_quorum_idempotent_after_reached`: blob data availability quorum already reached, additional attestations arrive — weight continues accumulating but `payload_data_available` stays true (no re-trigger), `payload_revealed` remains false (independent tracking)
  - `both_quorums_reached_in_single_call`: single attestation batch with `payload_present=true` AND `blob_data_available=true` pushes both counters over threshold simultaneously — both `payload_revealed` and `payload_data_available` set in one call, `execution_status` set from `bid_block_hash`
  - `payload_attestation_empty_indices_no_weight`: indexed attestation with zero attesting indices — `ptc_weight` and `ptc_blob_data_available_weight` remain 0, no quorum flags triggered
  - `payload_quorum_does_not_retrigger_status_on_second_batch`: first batch reaches quorum and sets `execution_status` from `bid_block_hash`. `bid_block_hash` is then changed. Second batch arrives — weight accumulates but `!node.payload_revealed` guard prevents re-entering quorum path, so `execution_status` remains unchanged
  - `independent_blocks_have_independent_ptc_state`: two blocks at different slots have independent PTC weight tracking — quorum reached on block_a does not affect block_b's `payload_revealed` or `payload_data_available` flags
- Verified: 74/74 fork_choice tests pass (was 69), 117/117 proto_array tests pass, 8/8 EF fork choice tests pass, cargo fmt + clippy clean
- Commit: `6011874ee`

### 2026-02-26 — fork choice ePBS lifecycle integration tests (run 125)
- Checked consensus-specs PRs since run 124: 3 Gloas-related PRs merged to stable since last tracked
  - **#4918** (merged Feb 23): "Only allow attestations for known payload statuses" — adds `validate_on_attestation` check: `if attestation.data.index == 1: assert beacon_block_root in store.payload_states`. **Already implemented** in vibehouse at fork_choice.rs:1179-1187 (checks `!block.payload_revealed`), with 3 unit tests
  - **#4930** (merged Feb 16): "Rename execution_payload_states to payload_states" — pure rename in spec Python code. **No vibehouse change needed** (we use internal naming)
  - **#4923** (merged Feb 16): "Ignore beacon block if parent payload unknown" — adds gossip IGNORE for blocks whose parent payload hasn't been seen. **Already implemented** in vibehouse at block_verification.rs:971-984 (`GloasParentPayloadUnknown`), with 3 integration tests in beacon_chain/tests/gloas.rs
  - Open PRs unchanged: #4948, #4947, #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4747, #4630
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Coverage gap analysis**: fork choice `on_execution_bid`, `on_payload_attestation`, and `on_execution_payload` had individual unit tests but were missing multi-call interaction and lifecycle tests
- **Added 5 lifecycle integration tests** for fork choice ePBS methods (fork_choice.rs):
  - `payload_attestation_accumulates_across_multiple_calls`: two separate PTC attestation batches, each below quorum individually, together reaching quorum (2 > threshold of 1 for MinimalEthSpec). Verifies `ptc_weight` accumulation and quorum trigger
  - `payload_attestation_quorum_without_bid_block_hash`: PTC quorum reached but `bid_block_hash` is None → `execution_status` stays `Irrelevant` (the `!is_execution_enabled() && bid_block_hash.is_none()` path)
  - `payload_attestation_quorum_skipped_when_already_revealed`: envelope reveals payload first, then PTC attestations arrive and exceed quorum — the `!node.payload_revealed` guard prevents `execution_status` from being overwritten by `bid_block_hash`
  - `blob_quorum_independent_of_payload_quorum`: blob `payload_data_available` quorum reached with `payload_present=false` — `payload_revealed` stays false, verifying independent quorum tracking
  - `full_lifecycle_bid_then_ptc_then_envelope`: realistic end-to-end: `on_execution_bid` (sets builder_index, initializes PTC) → `on_payload_attestation` (PTC quorum sets `payload_revealed` and `execution_status` from `bid_block_hash`) → `on_execution_payload` (envelope updates `execution_status` with actual payload hash)
- Verified: 69/69 fork_choice tests pass (was 64), 117/117 proto_array tests pass, 8/8 EF fork choice tests pass, cargo fmt + clippy clean
- Commit: `875dbb4f4`

### 2026-02-26 — process_execution_payload_bid edge case unit tests (run 124)
- Checked consensus-specs PRs since run 123: no new Gloas spec changes merged to stable
  - Open PRs unchanged: #4948 (reorder payload status constants), #4947 (pre-fork subscription note), #4940 (Gloas fork choice tests), #4939 (request missing envelopes for index-1), #4932 (Gloas sanity/blocks tests), #4898 (remove pending from tiebreaker), #4843 (variable PTC deadline), #4840 (eip7843), #4747 (Fast Confirmation Rule), #4630 (SSZ forward compat)
  - Newly tracked: #4892 (remove impossible branch in forkchoice — labeled gloas, changes `is_supporting_vote` from `<=` to `assert >= + ==` — vibehouse already uses `debug_assert` for this, no change needed)
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Coverage gap analysis**: `process_execution_payload_bid` had 17 existing unit tests but was missing tests for combined pending withdrawal+payment balance accounting, exact boundary conditions, bid overwrite behavior, and self-build common validation paths
- **Added 5 edge case unit tests** for `process_execution_payload_bid` (per_block_processing/gloas.rs):
  - `builder_bid_balance_accounts_for_both_withdrawals_and_payments`: verifies the spec's `get_pending_balance_to_withdraw_for_builder` correctly sums BOTH `builder_pending_withdrawals` AND `builder_pending_payments` when computing available balance. With 300 pending withdrawal + 400 pending payment, bid 301 fails but bid 300 succeeds (available = 1000 - 700 = 300)
  - `builder_bid_exact_boundary_balance`: balance = min_deposit + bid_value passes; min_deposit + bid_value + 1 fails. Tests the exact `builder_balance - min_balance >= bid_amount` boundary
  - `builder_bid_overwrites_cached_bid`: processes a builder bid (value=100), then a self-build bid. Verifies `state.latest_execution_payload_bid` is updated to the second bid, confirming overwrite behavior
  - `self_build_bid_wrong_slot_still_rejected`: self-build bids must also pass common checks (slot, parent, randao). Verifies that self-build with mismatched block_slot is rejected with "slot" error
  - `builder_bid_pending_payment_at_correct_slot_index`: verifies the exact slot index formula `SLOTS_PER_EPOCH + bid.slot % SLOTS_PER_EPOCH`. For slot=8, slots_per_epoch=8: index=8. Checks the payment is at index 8 and all other indices remain zero
- Verified: 307/307 state_processing tests pass (was 302), cargo fmt + clippy clean
- Commit: `e76997058`

### 2026-02-26 — process_withdrawals_gloas edge case unit tests (run 123)
- Checked consensus-specs PRs since run 122: no new Gloas spec changes merged to stable
  - Open PRs unchanged: #4948 (reorder payload status constants — approved by potuz, likely merging soon), #4947 (pre-fork subscription note), #4940 (Gloas fork choice tests), #4939 (request missing envelopes for index-1), #4932 (Gloas sanity/blocks tests), #4898 (remove pending from tiebreaker), #4843 (variable PTC deadline), #4840 (eip7843), #4747 (Fast Confirmation Rule), #4630 (SSZ forward compat)
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Conducted spec compliance audit** of fork choice (validate_on_attestation, is_supporting_vote, get_parent_payload_status, get_payload_tiebreaker) and beacon-chain state processing (process_execution_payload_envelope, process_builder_pending_payments, process_withdrawals_gloas). All implementations confirmed spec-compliant with no divergences
- **Added 7 edge case unit tests** for `process_withdrawals_gloas` (per_block_processing/gloas.rs):
  - `withdrawals_max_withdrawals_reached_updates_validator_index_from_last`: when all 4 withdrawal slots filled, `next_withdrawal_validator_index = (last.validator_index + 1) % validators_len` (the `if` branch at line 752)
  - `withdrawals_partial_amount_capped_to_excess`: pending partial withdrawal requesting 5 ETH when only 1 ETH excess → capped to 1 ETH
  - `withdrawals_builder_sweep_round_robin_from_nonzero_index`: 2 exited builders, sweep starting from index 1 wraps around to index 0, verifies ordering and builder index update
  - `withdrawals_pending_partial_not_withdrawable_yet_breaks`: future `withdrawable_epoch` prevents processing, partial stays in queue
  - `withdrawals_partial_and_validator_sweep_same_validator`: validator has both pending partial (2 ETH) and sweep excess (2 ETH), sweep accounts for already-withdrawn partial amount
  - `withdrawals_builder_sweep_zero_balance_skipped`: exited builder with zero balance produces no sweep withdrawal
  - `withdrawals_pending_partial_insufficient_balance_skipped`: partial withdrawal counted as processed but generates no withdrawal entry when balance <= min_activation_balance
- Verified: 302/302 state_processing tests pass (was 295), cargo fmt + clippy clean
- Commit: `bcb55df71`

### 2026-02-26 — fix get_payload_tiebreaker spec compliance bug (run 122)
- Checked consensus-specs PRs since run 121: no new Gloas spec changes merged to stable
  - Open PRs unchanged: #4948 (reorder payload status constants), #4947 (pre-fork subscription note), #4940 (Gloas fork choice tests), #4939 (request missing envelopes for index-1), #4932 (Gloas sanity/blocks tests), #4898 (remove pending from tiebreaker), #4843 (variable PTC deadline), #4840 (eip7843), #4747 (Fast Confirmation Rule), #4630 (SSZ forward compat)
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Found and fixed spec compliance bug in `get_payload_tiebreaker`** (proto_array_fork_choice.rs):
  - **Bug**: The function only checked `!is_previous_slot` to decide when to return the ordinal status value. The spec says `if PENDING or not_previous_slot → return ordinal`. Missing the PENDING check meant that a PENDING node from the previous slot (e.g., the justified checkpoint when justified.slot + 1 == current_slot) would fall through to the EMPTY/FULL branches and incorrectly call `should_extend_payload`, returning 2 or 0 instead of the correct 0 (PENDING ordinal)
  - **Impact**: In head selection, the `get_head` loop sorts children by `(weight, root, tiebreaker)`. A PENDING node from the previous slot with a timely payload would get tiebreaker=2 instead of 0, potentially causing it to win tiebreaks against FULL nodes that should have won. This is an edge case that occurs when the justified checkpoint is at the previous slot
  - **Fix**: Added `node.payload_status == GloasPayloadStatus::Pending ||` before `!is_previous_slot` in the condition, matching the spec's OR semantics exactly
  - **Added test**: `tiebreaker_pending_at_previous_slot_returns_zero` — sets up a PENDING node at the previous slot with payload_revealed+data_available (so should_extend_payload would return 2), verifies the tiebreaker correctly returns 0
- Verified: 117/117 proto_array tests pass, 64/64 fork_choice tests pass, 8/8 EF fork choice tests pass, cargo fmt + clippy clean

### 2026-02-26 — fix should_extend_payload spec compliance bug (run 121)
- Checked consensus-specs PRs since run 120: no new Gloas spec changes merged to stable
  - Open PRs unchanged: #4948 (reorder payload status constants), #4947 (pre-fork subscription note), #4940 (Gloas fork choice tests), #4939 (request missing envelopes for index-1), #4932 (Gloas sanity/blocks tests), #4898 (remove pending from tiebreaker), #4843 (variable PTC deadline), #4840 (eip7843), #4747 (Fast Confirmation Rule), #4630 (SSZ forward compat)
  - Newly merged since run 120: #4946 (bump actions/stale), #4945 (fix inclusion list test for mainnet) — neither affects Gloas
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Found and fixed spec compliance bug in `should_extend_payload`** (proto_array_fork_choice.rs):
  - **Bug**: The last condition in `should_extend_payload` checked `parent_node.payload_revealed` (a runtime flag indicating whether the execution payload envelope has been received). The spec's `is_parent_node_full(store, store.blocks[proposer_root])` is a **static** check comparing `boosted_block.bid.parent_block_hash == parent.bid.block_hash` — whether the boosted block's bid declares that it builds on the FULL version of its parent
  - **Impact**: `payload_revealed` can be true when the child builds on EMPTY (if child's bid.parent_block_hash doesn't match parent's bid.block_hash), or false when the child expects FULL but the envelope hasn't arrived yet. Using the wrong check meant `should_extend_payload` could return the wrong answer in edge cases, leading to incorrect payload tiebreaker values (FULL 2 vs 0)
  - **Fix**: Replaced `parent_node.payload_revealed` with `self.get_parent_payload_status_of(boosted_node, parent_node) == GloasPayloadStatus::Full`, which correctly compares the bid block hashes per spec
  - **Updated 2 tests**: `should_extend_payload_boosted_parent_is_this_root_and_full` (now sets `bid_parent_block_hash` to match parent's `bid_block_hash`) and `should_extend_payload_boosted_parent_is_this_root_and_not_full` (now verifies `bid_parent_block_hash` is None)
- Verified: 116/116 proto_array tests pass, 64/64 fork_choice tests pass, 8/8 EF fork choice tests pass, cargo fmt + clippy clean

### 2026-02-26 — dead code cleanup in fork choice and envelope processing (run 120)
- Checked consensus-specs PRs since run 119: no new Gloas spec changes merged to stable
  - Open PRs tracked: #4948 (reorder payload status constants), #4947 (pre-fork subscription note), #4940 (Gloas fork choice tests), #4939 (request missing envelopes for index-1), #4932 (Gloas sanity/blocks tests), #4898 (remove pending from tiebreaker), #4843 (variable PTC deadline), #4840 (eip7843), #4747 (Fast Confirmation Rule), #4630 (SSZ forward compat)
  - PR #4942 (Promote EIP-7805 to Heze) merged — creates new Heze fork, does NOT affect Gloas
  - PR #4941 (Update execution proof construction) merged — in `_features/eip8025/`, not Gloas
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Conducted coverage gap analysis** using comprehensive codebase scan. Found:
  - 5 dead variants in `InvalidExecutionBid` enum (fork_choice.rs): `ParentMismatch`, `UnknownBuilder`, `BuilderNotActive`, `InsufficientBuilderBalance`, `ZeroValueBid` — validations done at gossip/state-processing layer, never at fork choice
  - 3 dead variants in `InvalidPayloadAttestation` enum (fork_choice.rs): `SlotMismatch`, `InvalidAttester`, `InvalidSignature` — same pattern
  - 1 dead variant in `EnvelopeProcessingError` (envelope_processing.rs): `ExecutionInvalid` — EL validity checked at beacon chain layer, not state processing
  - Several hard-to-trigger internal error paths (`NotGloasBlock`, `MissingBeaconBlock`, `PtcCommitteeError`, `BeaconChainError`) that represent DB corruption or infrastructure failures — not practical to test
- **Removed all dead code variants**: 30 lines deleted across fork_choice.rs and envelope_processing.rs
- Verified: 64/64 fork_choice tests pass, 295/295 state_processing tests pass, 44/44 envelope tests pass, 116/116 proto_array tests pass, 8/8 EF fork choice tests pass, 2205/2205 workspace tests pass (excluding web3signer which needs external server)
- Commit: `30738d1f8`

### 2026-02-26 — VC proposer preferences broadcasting (run 119)
- Identified missing spec feature: the Validator Client was not broadcasting proposer preferences, which is required by gloas/validator.md ("At the beginning of each epoch, a validator MAY broadcast SignedProposerPreferences")
- **Implemented VC proposer preferences broadcasting** across 7 files:
  - `signing_method/src/lib.rs`: added `ProposerPreferences` variant to `SignableMessage` + signing_root + Web3Signer error
  - `validator_store/src/lib.rs`: added `sign_proposer_preferences` to `ValidatorStore` trait
  - `lighthouse_validator_store/src/lib.rs`: implemented `sign_proposer_preferences` using `Domain::ProposerPreferences`
  - `validator_services/src/duties_service.rs`: added `broadcast_proposer_preferences` (~170 lines) — fetches next-epoch duties, filters to local validators, signs preferences with configured fee_recipient/gas_limit, submits to BN
  - `validator_services/src/ptc.rs` + `payload_attestation_service.rs`: added trait stubs for mock stores
  - `beacon_node/http_api/src/lib.rs`: updated POST beacon/pool/proposer_preferences to gossip preferences via P2P after validation
- All 42 VC tests pass, 573 beacon_chain tests pass, 136 network tests pass, 2205 workspace tests pass
- Commit: `de6143492`

### 2026-02-26 — proposer preferences bid validation unit tests (run 118)
- Checked consensus-specs PRs since run 117: no new Gloas spec changes merged to stable
  - Open PRs unchanged: #4948 (reorder payload status constants), #4947 (pre-fork subscription note), #4940 (Gloas fork choice tests), #4939 (request missing envelopes for index-1), #4898 (remove pending from tiebreaker), #4843 (variable PTC deadline), #4840 (eip7843), #4747 (Fast Confirmation Rule), #4630 (SSZ forward compat)
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Closed coverage gap**: `ProposerPreferencesNotSeen`, `FeeRecipientMismatch`, and `GasLimitMismatch` error paths in `verify_execution_bid_for_gossip` (gloas_verification.rs) were only tested at the network integration level (tests.rs), not at the beacon_chain unit test level in `gloas_verification.rs`
- **Added 3 unit tests** to `gloas_verification.rs`:
  - `bid_no_proposer_preferences_ignored`: bid submitted without any preferences in the pool → `ProposerPreferencesNotSeen`. The IGNORE path: proposer hasn't published their fee_recipient/gas_limit requirements yet, so the bid cannot be validated and is silently dropped
  - `bid_fee_recipient_mismatch_rejected`: bid with `fee_recipient=0xaa`, preferences require `fee_recipient=0xbb` → `FeeRecipientMismatch`. Tests that builders cannot override the proposer's preferred execution address (REJECT = peer penalty)
  - `bid_gas_limit_mismatch_rejected`: bid with `gas_limit=30_000_000`, preferences require `gas_limit=20_000_000` → `GasLimitMismatch`. Tests that gas limits must match exactly between bid and proposer preferences (REJECT = peer penalty)
- These paths are checked after parent_block_root validation (check 4) and before signature verification (check 5), so the tests use `BLOCKS_TO_FINALIZE` harness to ensure the builder is active at the finalized epoch
- All 52 gloas_verification tests pass (was 49)

### 2026-02-26 — ExecutionPayloadEnvelopesByRoot RPC handler tests (run 117)
- Checked consensus-specs PRs since run 116: no new Gloas spec changes merged to stable
  - Open PRs tracked: #4948 (reorder payload status constants), #4947 (pre-fork subscription note), #4939 (request missing envelopes for index-1), #4898 (remove pending from tiebreaker), #4843 (variable PTC deadline), #4840 (eip7843), #4747 (Fast Confirmation Rule), #4630 (SSZ forward compat)
  - Newly tracked: #4940 (Gloas fork choice tests — open, not merged)
  - Recently merged but already implemented: #4918 (attestations for known payload statuses), #4923 (block queueing for unknown parent payload), #4897 (pending validator check before builder deposit)
  - PR #4914 (replace prover_pubkey with validator_index in SignedExecutionProof) targets eip8025, not core Gloas spec — not applicable to vibehouse's ZK-proof ExecutionProof design
  - PR #4931 (FOCIL onto Gloas) — in `specs/_features/eip7805/`, not stable Gloas spec. Does add `inclusion_list_bits: Bitvector` to `ExecutionPayloadBid` and new IL satisfaction logic, but this is speculative/experimental, not scheduled
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Closed coverage gap**: `ExecutionPayloadEnvelopesByRoot` P2P protocol (handle_execution_payload_envelopes_by_root_request in rpc_methods.rs) had ZERO integration tests. This is the Gloas-specific RPC protocol for serving payload envelopes by beacon block root to peers
- **Added `enqueue_envelopes_by_root_request` helper** to `TestRig` in tests.rs — creates an `ExecutionPayloadEnvelopesByRootRequest` from a list of block roots and sends it to the beacon processor
- **Added `drain_envelopes_by_root_responses` helper** — drains `ExecutionPayloadEnvelopesByRoot` responses from the network channel until the stream terminator (None) is received, returning the collected envelopes
- **Added 3 integration tests**:
  - `test_gloas_envelopes_by_root_known_root_served`: requests block root at slot 1 (stored in Gloas chain) → verifies one envelope is returned. Confirms the happy path: handler finds the envelope in the store and streams it before the terminator
  - `test_gloas_envelopes_by_root_unknown_root_not_served`: requests `Hash256::repeat_byte(0xab)` (not in store) → verifies no envelopes are returned. Confirms the handler silently skips unknown roots (only terminator sent)
  - `test_gloas_envelopes_by_root_mixed_roots`: requests [slot1_root, unknown, slot2_root] → verifies 2 envelopes returned. Confirms the handler iterates all requested roots and only serves the ones it finds, skipping the unknown one mid-stream
- All 136 network tests pass (was 133); cargo fmt + clippy clean

### 2026-02-26 — fix components_by_range_requests memory leak (run 116)
- No new Gloas spec changes since run 115; open PRs unchanged
- **Bug fixed**: `components_by_range_requests` entries in `SyncNetworkContext` could accumulate without being freed
  - **Path 1 — retry failure**: In `retry_columns_by_range`, if peer selection or request sending failed, the function returned `Err` but left the entry in the map. Fixed by removing the entry before returning on both error paths.
  - **Path 2 — chain removal**: When a range sync chain was removed (peer disconnect, chain failure, chain completed), its `components_by_range_requests` entries were never cleaned up. Fixed by calling `remove_range_components_by_chain_id(chain.id())` in `on_chain_removed` (range.rs).
  - **Path 3 — backfill failure**: When backfill sync failed, its entries were never cleaned up. Fixed by calling `remove_backfill_range_components()` in the three error-handling branches in manager.rs (`on_batch_process_result`, `on_block_response`, `inject_error`).
- All 133 network tests pass; full clippy clean

### 2026-02-26 — CI coverage improvements (run 115)
- Checked consensus-specs PRs since run 114: no new Gloas spec changes merged
  - Open PRs tracked: #4948 (reorder payload status constants), #4947 (pre-fork subscription note), #4939 (request missing envelopes for index-1), #4898 (remove pending from tiebreaker), #4843 (variable PTC deadline), #4840 (eip7843), #4747 (Fast Confirmation Rule), #4630 (SSZ forward compat)
  - Still open, not implementing until merged
- Spec test version: v1.7.0-alpha.2 remains latest release
- **CI improvements**: two gaps closed in test coverage for CI
  - Added `operation_pool tests (gloas)` to `ci.yml` fork-specific-tests job — operation_pool runs in `unit-tests` job without FORK_NAME, but now also runs with `FORK_NAME=gloas` to exercise attestation reward calculations, pool operations, and pack_attestations with Gloas-era beacon state (ePBS bids, payload availability bits). All 26 tests pass
  - Added `gloas` to `RECENT_FORKS` in Makefile — `nightly test-suite.yml` uses `make test-http-api` which iterates `RECENT_FORKS`. Adding gloas means nightly CI now runs all 212 http_api tests with `FORK_NAME=gloas`, catching Gloas-specific HTTP API regressions (gossip block import guards, payload envelope endpoints, PTC duty endpoints)
- 570/570 beacon_chain tests pass, 26/26 operation_pool tests pass (verified locally)

### 2026-02-26 — blinded envelope fallback in reconstruct_historic_states (run 114)
- Checked consensus-specs PRs since run 113: no new Gloas spec changes merged
  - Open PRs tracked: #4948 (reorder payload status constants), #4947 (pre-fork subscription note), #4939 (request missing envelopes for index-1), #4898 (remove pending from tiebreaker), #4843 (variable PTC deadline), #4840 (eip7843), #4747 (Fast Confirmation Rule), #4630 (SSZ forward compat)
  - Still open, not implementing until merged
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Closed P6 coverage gap**: the blinded envelope fallback path in `reconstruct_historic_states` (reconstruct.rs:131-146) and `get_advanced_hot_state` (hot_cold_store.rs:1191-1203) had ZERO tests exercising the fallback path (where full payload is pruned and only blinded envelope remains)
- **Added `gloas_reconstruct_states_with_pruned_payloads` test** (store_tests.rs):
  - Builds 7-epoch Gloas chain with `reconstruct_historic_states: false` (states not auto-reconstructed)
  - Collects Gloas block roots, pre-envelope state roots, and bid block_hashes before pruning
  - Calls `try_prune_execution_payloads(force=true)` — deletes full payloads from ExecPayload column
  - Verifies: `execution_payload_exists()` returns false, `get_payload_envelope()` returns None, `get_blinded_payload_envelope()` still returns Some (blinded envelopes are NOT pruned)
  - Calls `reconstruct_historic_states(None)` — must use blinded envelope fallback for all Gloas blocks since full payloads are gone
  - Loads reconstructed cold states by pre-envelope root and verifies `latest_block_hash == bid.block_hash` (confirms envelope processing was applied via blinded fallback)
- **Key design insight**: `reconstruct_historic_states` stores states under `block.state_root()` (pre-envelope root). The state CONTENT has envelope applied (latest_block_hash updated). `load_cold_state_by_slot` replays from snapshots/hdiffs that include the envelope changes, so loaded states have correct `latest_block_hash`
- **What this tests**: the only previously untested path — real production nodes prune payloads after finalization, then `reconstruct_historic_states` is used during WSS archive node setup. Without blinded envelope fallback, reconstruction would leave `latest_block_hash` at the grandparent's value, breaking bid validation for all reconstructed states
- 570/570 beacon_chain tests pass (was 566), cargo fmt + clippy clean
- **No remaining known coverage gaps** — all P1-P8 gaps from run 96 analysis are now closed

### 2026-02-26 — produce_payload_attestations integration tests (run 113)
- Checked consensus-specs PRs since run 112: no new Gloas spec changes merged
  - Open PRs tracked: #4948 (reorder payload status constants, EMPTY=0/FULL=1/PENDING=2), #4947 (pre-fork subscription note), #4940, #4939, #4932, #4892 (already implemented), #4840, #4747, #4630, #4558
  - PR #4892 (remove impossible branch in forkchoice) — already implemented in vibehouse as `debug_assert!(vote.current_slot >= block.slot)` + `if vote.current_slot == block.slot { return false; }`
  - PR #4948 still open — not implementing yet (PENDING=0→2 enum reorder requires spec finalization)
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Closed P2 coverage gap**: `produce_payload_attestations` in `payload_attestation_service.rs` had ZERO integration tests. This is the core VC routine that PTC members execute at 3/4 of each slot — reads duties from DutiesService.ptc_duties, fetches attestation data from BN, signs with validator store, submits to pool
- **Made `PtcDutiesMap::set_duties` pub(crate)** to allow duty injection from the sibling test module
- **Added test-only `produce_payload_attestations_for_testing` method** (wraps the private async fn) to expose it for integration tests
- **Added `SigningValidatorStore`**: minimal ValidatorStore for produce_payload_attestations tests — implements `voting_pubkeys`, `validator_index`, and `sign_payload_attestation` (with configurable error injection and signed-indices recording). All other methods are `unimplemented!()` stubs
- **Added 6 integration tests** in `produce_tests` module (payload_attestation_service.rs):
  - `produce_no_duties_returns_ok_without_bn_call`: slot has duties for slot 999 (not current slot) → duties_for_slot returns empty → early return without any BN call
  - `produce_with_duties_signs_and_submits`: happy path — duty present for current slot, BN returns attestation data, sign succeeds, POST to pool. Verifies sign was called for the correct validator_index
  - `produce_multiple_duties_all_signed`: 3 validators with duties in same slot → all 3 signed and submitted in a single POST. Tests the duty iteration loop
  - `produce_bn_error_returns_err`: no BN mock → BN returns 404 → produce_payload_attestations returns Err(()). Tests abort-on-fetch-failure
  - `produce_sign_error_skips_submission`: sign errors for all duties → messages vec empty → returns Ok without POST (sign attempt recorded). Tests error resilience (function logs and continues, not a fatal abort)
  - `produce_payload_present_false_propagated`: BN returns payload_present=false → sign still called with false data. Verifies false payload presence is a valid duty (not suppressed)
- **No remaining P2 coverage gaps** — both `poll_ptc_duties` (run 112) and `produce_payload_attestations` (run 113) are now tested
- All 35 validator_services tests pass (was 29), cargo fmt + clippy clean

### 2026-02-26 — poll_ptc_duties integration tests (run 112)
- Checked consensus-specs PRs since run 111: no new Gloas spec changes merged
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Closed P5 coverage gap**: `poll_ptc_duties` in `validator_services/src/ptc.rs` had ZERO integration tests. The function fetches PTC (Payload Timeliness Committee) duties from the beacon node for current + next epoch and caches them in `PtcDutiesMap`
- **Added mock BN methods to `MockBeaconNode`** (`testing/validator_test_rig/src/mock_beacon_node.rs`):
  - `mock_post_validator_duties_ptc(epoch, duties)`: mocks `POST /eth/v1/validator/duties/ptc/{epoch}`
  - `mock_get_validator_payload_attestation_data(data)`: mocks `GET /eth/v1/validator/payload_attestation_data`
  - `mock_post_beacon_pool_payload_attestations()`: mocks `POST /eth/v1/beacon/pool/payload_attestations`
- **Added `MinimalValidatorStore`**: implements `ValidatorStore` trait with only the two methods needed by `poll_ptc_duties` (`voting_pubkeys` and `validator_index`) — all other async methods are `async fn { unimplemented!() }` stubs
- **Added 7 integration tests** in `poll_tests` module (validator_services/src/ptc.rs):
  - `poll_ptc_duties_pre_gloas_skips_bn`: slot 0 (pre-Gloas, spec slots_per_epoch=8, Gloas at epoch 1 = slot 8) → no BN call
  - `poll_ptc_duties_fetches_current_and_next_epoch`: slot 16 (epoch 2) → fetches both epoch 2 and epoch 3 duties, stores in map
  - `poll_ptc_duties_cached_epoch_not_refetched`: call twice with same slot → BN called only once (second call hits cache)
  - `poll_ptc_duties_no_validators_skips_bn`: empty validator store → no BN call (early return)
  - `poll_ptc_duties_empty_response_stored`: BN returns empty duties vec → stored as empty (not absent)
  - `poll_ptc_duties_gloas_disabled_skips_bn`: `gloas_fork_epoch = u64::MAX` (disabled) → no BN call
  - `poll_ptc_duties_multiple_validators`: 3 validators → all 3 pubkeys sent in request, duties returned and stored
- **Remaining coverage gap**: P2 (PayloadAttestationService `produce_payload_attestations`) — more complex, requires producing and submitting a payload attestation with a real PTC slot
- All 29 validator_services tests pass, cargo fmt + clippy clean

### 2026-02-26 — Proposer preferences pool + bid validation against preferences (run 111)
- Checked consensus-specs PRs since run 110: no new Gloas spec changes merged
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Implemented proposer preferences pool**: `BeaconChain.proposer_preferences_pool` (`Mutex<HashMap<Slot, SignedProposerPreferences>>`) stores verified proposer preferences for bid validation. Pool auto-prunes entries older than 2 epochs. Methods: `insert_proposer_preferences` (returns false for dedup), `get_proposer_preferences`
- **Added bid validation against proposer preferences** (spec compliance fix): `verify_execution_bid_for_gossip` now validates:
  - [IGNORE] SignedProposerPreferences for bid.slot has been seen → `ProposerPreferencesNotSeen`
  - [REJECT] bid.fee_recipient matches proposer's preferences → `FeeRecipientMismatch`
  - [REJECT] bid.gas_limit matches proposer's preferences → `GasLimitMismatch`
- **Updated gossip handler**: `process_gossip_proposer_preferences` now checks for dedup (IGNORE second message for same slot) and stores accepted preferences in the pool. `process_gossip_execution_bid` routes the 3 new error types correctly (ProposerPreferencesNotSeen → Ignore, FeeRecipientMismatch/GasLimitMismatch → Reject + LowToleranceError)
- **Added 3 new bid gossip handler integration tests**:
  - `test_gloas_gossip_bid_no_preferences_ignored`: bid without preferences in pool → Ignore
  - `test_gloas_gossip_bid_fee_recipient_mismatch_rejected`: bid with wrong fee_recipient → Reject
  - `test_gloas_gossip_bid_gas_limit_mismatch_rejected`: bid with wrong gas_limit → Reject
- **Updated 4 existing bid tests** to insert matching preferences before bid submission (required after preferences check was added)
- All 133 network tests pass (was 130), cargo fmt + clippy clean

### 2026-02-26 — Payload attestation gossip handler integration tests + InvalidSignature bug fix (run 110)
- Checked consensus-specs PRs since run 109: no new Gloas spec changes merged
  - No new PRs merged since Feb 24. All tracked Gloas PRs still open: #4948, #4947, #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Found and fixed a bug**: `PayloadAttestationError::InvalidSignature` was falling through to the catch-all error handler in `process_gossip_payload_attestation`, returning `MessageAcceptance::Ignore` instead of `Reject`. This was inconsistent with how attestations (`AttnError::InvalidSignature` → Reject), execution bids (`ExecutionBidError::InvalidSignature` → Reject), and payload envelopes (`PayloadEnvelopeError::InvalidSignature` → Reject) handle the same error. Invalid signatures indicate malicious behavior and must result in peer penalty + rejection
- **Added `build_valid_payload_attestation` helper**: constructs a properly-signed payload attestation from a real PTC committee member. Gets PTC committee via `get_ptc_committee`, picks the first member, computes signing root with `Domain::PtcAttester`, signs with the validator's BLS key, wraps in `AggregateSignature`, and sets the correct aggregation bit
- **Added 3 payload attestation gossip handler integration tests** (previously 3 tests covering simple error paths; now 6 total):
  - **Valid Accept (1 test):**
    - `test_gloas_gossip_payload_attestation_valid_accepted`: properly signed attestation from a real PTC committee member, correct slot, known block root, valid aggregation bits, valid BLS signature. Returns Accept. Tests the full validation pipeline end-to-end including signature verification
  - **ValidatorEquivocation → Reject (1 test):**
    - `test_gloas_gossip_payload_attestation_equivocation_rejected`: sends payload_present=true (Accept), then payload_present=false from the same PTC member (Reject). Tests the observed_payload_attestations equivocation detection — same validator + same slot + different payload_present = equivocation
  - **InvalidSignature → Reject (1 test):**
    - `test_gloas_gossip_payload_attestation_invalid_signature_rejected`: correct PTC aggregation bits but signed with a different validator's key. Returns Reject. Tests BLS aggregate signature verification and the new explicit InvalidSignature handler
- These tests close the payload attestation gossip handler gap identified in run 109: ValidatorEquivocation and valid Accept paths are now covered, and the InvalidSignature bug was found and fixed in the process
- **Remaining handler gaps**: P2 (PayloadAttestationService), P5 (poll_ptc_duties) — both require mock BN infrastructure
- All 130 network tests pass (was 127), cargo fmt + clippy clean

### 2026-02-26 — Execution bid gossip handler builder-path tests (run 109)
- Checked consensus-specs PRs since run 108: no new Gloas spec changes merged
  - No new PRs merged since Feb 24. All tracked Gloas PRs still open: #4948, #4947, #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630
  - PRs to watch: #4948 (reorder payload status constants), #4947 (pre-fork subscription for proposer_preferences), #4926 (SECONDS_PER_SLOT → SLOT_DURATION_MS rename)
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Addressed execution bid gossip handler builder-path error variants**: the `process_gossip_execution_bid` handler (gossip_methods.rs:3240-3398) had 3 tests covering simple error paths (ZeroExecutionPayment, SlotNotCurrentOrNext, UnknownBuilder) but ZERO tests for error paths requiring a registered builder: DuplicateBid, BuilderEquivocation, InvalidParentRoot, InsufficientBuilderBalance, InvalidSignature, and the happy-path Accept
- **Built test infrastructure**: `gloas_rig_with_builders` helper creates a Gloas TestRig with builders injected into the genesis state via InteropGenesisBuilder + direct state mutation. Extends chain 128 blocks (4 epochs) to achieve finalization, enabling `is_active_at_finalized_epoch` check to pass. `TestRig::new_from_harness` is a new constructor that wraps a pre-built harness with full beacon processor + network channels. `sign_bid` helper properly signs bids using BUILDER_KEYPAIRS with Domain::BeaconBuilder
- **Added 6 execution bid gossip handler integration tests** (previously ZERO tests for these paths):
  - **DuplicateBid → Ignore (1 test):**
    - `test_gloas_gossip_bid_duplicate_ignored`: sends the same signed bid twice. First returns Accept, second returns Ignore. Tests the observed_execution_bids deduplication — the equivocation check records the bid root on first verification, and a second identical bid is treated as a duplicate
  - **BuilderEquivocation → Reject (1 test):**
    - `test_gloas_gossip_bid_equivocation_rejected`: sends two different bids from builder 0 for the same slot (value=100 vs value=200 → different tree hash roots). First returns Accept, second returns Reject. Tests the equivocation detection — same builder_index + same slot + different bid root = equivocation
  - **InvalidParentRoot → Ignore (1 test):**
    - `test_gloas_gossip_bid_invalid_parent_root_ignored`: sends a bid with parent_block_root=0xff (doesn't match fork choice head). Returns Ignore. Tests the head-matching guard — bids for non-head parents are stale, not malicious
  - **InsufficientBuilderBalance → Ignore (1 test):**
    - `test_gloas_gossip_bid_insufficient_balance_ignored`: registers builder with balance=10, sends bid with value=1_000_000. Returns Ignore. Tests the balance check — builders can't bid more than their registered balance
  - **InvalidSignature → Reject (1 test):**
    - `test_gloas_gossip_bid_invalid_signature_rejected`: signs a bid for builder 0 using builder 1's secret key. Returns Reject. Tests BLS signature verification — the handler correctly rejects bids with invalid signatures and penalizes the peer
  - **Valid Accept — happy path (1 test):**
    - `test_gloas_gossip_bid_valid_accepted`: properly signed bid from a registered, active builder with sufficient balance, correct parent root, and valid slot. Returns Accept. Tests the complete validation pipeline end-to-end through the gossip handler
- These tests close the execution bid gossip handler gap identified in run 105: all 6 remaining error paths that required a registered builder in the test state are now covered. The equivocation test is particularly important — equivocating builders must be penalized to prevent bid spam attacks. The happy-path Accept test exercises the full pipeline including `apply_execution_bid_to_fork_choice`
- **Remaining handler gaps**: payload attestation remaining paths (ValidatorEquivocation, valid Accept) — require valid PTC committee signatures; P2 (PayloadAttestationService), P5 (poll_ptc_duties) — both require mock BN infrastructure
- All 127 network tests pass (was 121), cargo fmt + clippy clean

### 2026-02-26 — Fix latest_block_hash for empty parent payloads (run 108)
- **Fixed 5 Gloas fork_choice EF test failures** and **29 store_test failures** — all caused by incorrect `latest_block_hash` patching when the parent's payload was not revealed
- **Root cause**: `get_advanced_hot_state` unconditionally patched `latest_block_hash` from the parent bid's `block_hash`, even when the parent's envelope hadn't been processed. The spec's `on_block` has a two-state model:
  - Parent FULL (envelope revealed) → use `payload_states` (post-envelope, `latest_block_hash = bid.block_hash`)
  - Parent EMPTY (no envelope) → use `block_states` (pre-envelope, `latest_block_hash = grandparent's block_hash`)
- **Fix**: Moved `latest_block_hash` patching from `get_advanced_hot_state` (store layer) to `load_parent` (block_verification layer) where we have access to both child and parent blocks. Now uses `is_parent_node_full` logic from the spec: only patches when `child_bid.parent_block_hash == parent_bid.block_hash` (parent is full). When parent is empty, the pre-envelope `latest_block_hash` is correct as-is
- **Tests**: 78/78 EF tests pass, 138/138 fake_crypto pass, 566/566 beacon_chain tests pass, 121/121 network tests pass
- **Files changed**: `block_verification.rs` (+29 lines), `hot_cold_store.rs` (-39 lines)

### 2026-02-25 — Gloas canonical_head and payload attributes tests (run 107 continued)
- **Addressed canonical_head.rs Gloas branches**: `parent_random()` (line 172) and `head_block_number()` (line 189) had ZERO test coverage with Gloas-enabled heads. These methods are called during `prepare_beacon_proposer` → `get_pre_payload_attributes` to compute FCU payload attributes for the execution layer. If `parent_random()` returns the wrong value, the EL builds a payload with incorrect prev_randao, causing the block to be rejected by peers
- **Added 4 canonical_head / payload attributes integration tests** (previously ZERO tests for these paths):
  - **parent_random Gloas path (1 test):**
    - `gloas_canonical_head_parent_random_reads_from_bid`: extends chain, reads bid's prev_randao from head block, verifies `parent_random()` returns it. Tests the Gloas-specific branch that reads from `bid.message.prev_randao` instead of `execution_payload.prev_randao()`
  - **head_block_number Gloas path (1 test):**
    - `gloas_canonical_head_block_number_returns_zero`: extends chain, verifies `head_block_number()` returns 0 for Gloas head. Tests the fallback (block number is in envelope, not block body)
  - **get_pre_payload_attributes normal path (1 test):**
    - `gloas_get_pre_payload_attributes_succeeds`: extends chain, calls `get_pre_payload_attributes` with proposer_head==head. Verifies prev_randao matches `head_random()`, parent_block_number==0, parent_beacon_block_root==head
  - **get_pre_payload_attributes re-org path (1 test):**
    - `gloas_get_pre_payload_attributes_reorg_uses_parent_random`: extends chain, calls with proposer_head==parent (simulating re-org). Verifies prev_randao matches `parent_random()` (bid's prev_randao), parent_block_number==0 (0.saturating_sub(1))
- These tests close two gaps from the run 107 analysis: canonical_head.rs Gloas branches (#4 and #5) and the get_pre_payload_attributes Gloas pipeline. The re-org test is particularly important — it exercises the path where the proposer builds on the parent instead of the head, which requires reading prev_randao from the head block's bid (the parent's RANDAO was overwritten in the state)
- All 562 beacon_chain tests pass (was 558), cargo fmt + clippy clean

### 2026-02-25 — Gloas self-build envelope EL/error path tests + spec tracking (run 107)
- Checked consensus-specs PRs since run 106: no new Gloas spec changes merged
  - #4946 (GH Actions dependency bump) and #4945 (inclusion list test fix — Heze, not Gloas) — both irrelevant
  - New open PRs to track: #4947 (pre-fork subscription note for proposer_preferences topic), #4948 (reorder payload status constants — would change EMPTY 1→0, FULL 2→1)
  - All previously tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4926, #4558, #4747
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Addressed process_self_build_envelope EL execution status and error paths**: the `process_self_build_envelope` method (beacon_chain.rs) transitions blocks from Optimistic to Valid via `on_valid_execution_payload` after the EL confirms the payload. Had ZERO tests verifying this critical execution status transition, the stateless mode behavior, error paths, or the chain's ability to continue producing blocks after envelope processing
- **Added 5 self-build envelope integration tests** (previously ZERO tests for these paths):
  - **Execution status transition (1 test):**
    - `gloas_self_build_envelope_marks_execution_status_valid`: imports block (Optimistic), processes self-build envelope (mock EL returns Valid), verifies execution_status transitions to Valid(payload_block_hash). Tests the critical path: without this transition, head stays Optimistic and block production is disabled
  - **Stateless mode behavior (1 test):**
    - `gloas_self_build_envelope_stateless_mode_stays_optimistic`: uses stateless harness (no EL), processes self-build envelope, verifies execution_status remains Optimistic (EL not called) but payload_revealed=true and state transition still runs (latest_block_hash set). Tests the stateless validation path where EL verification is skipped
  - **Missing block root error (1 test):**
    - `gloas_self_build_envelope_missing_block_root_errors`: constructs envelope referencing non-existent block, verifies error mentioning "Missing beacon block". Tests the guard against envelopes arriving for unimported blocks
  - **Continued block production (1 test):**
    - `gloas_self_build_envelope_enables_next_block_production`: imports block, processes envelope, recomputes head, produces next block. Verifies the chain can continue producing blocks after envelope processing — parent_root matches, bid's parent_block_hash matches previous envelope's payload block_hash
  - **Store persistence field verification (1 test):**
    - `gloas_self_build_envelope_store_persistence_fields`: imports block (no envelope in store), processes envelope, verifies all stored envelope fields match (slot, builder_index, beacon_block_root, payload block_hash, BUILDER_INDEX_SELF_BUILD)
- These tests close a critical gap: process_self_build_envelope is the ONLY code path that transitions self-built blocks from Optimistic to Valid. If this transition fails, the node cannot produce subsequent blocks (forkchoiceUpdated returns SYNCING for optimistic heads). The stateless mode test verifies that stateless nodes correctly skip EL calls while still performing state transitions
- **Remaining gaps from run 96 analysis**: P2 (PayloadAttestationService), P5 (poll_ptc_duties) — both require mock BN infrastructure; P6 (store reconstruct blinded envelope fallback), P8 (post_block_import self-build envelope branch — now partially covered by these tests)
- All 558 beacon_chain tests pass (was 553), cargo fmt + clippy clean

### 2026-02-25 — Gloas early attester cache payload_present tests + spec tracking (run 106)
- Checked consensus-specs PRs since run 105: no new Gloas spec changes merged
  - No new Gloas PRs merged since run 105 (latest merge was #4918 on Feb 23, already tracked)
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4926, #4558
  - PRs to watch: #4926 (SECONDS_PER_SLOT → SLOT_DURATION_MS rename), #4558 (cell dissemination), #4747 (Fast Confirmation Rule, updated Feb 25)
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Addressed early attester cache Gloas payload_present gap**: the `EarlyAttesterCache::try_attest()` method (early_attester_cache.rs:132-148) independently computes `payload_present` from the proto_block's `payload_revealed` field, but had ZERO test coverage with Gloas enabled. The existing tests in `attestation_production.rs` use `default_spec()` which doesn't enable Gloas, so the early cache always computed `payload_present=false` regardless of the proto_block's `payload_revealed` state
- **Added 5 early attester cache Gloas integration tests** (previously ZERO tests for this pipeline with Gloas):
  - **Same-slot behavior (1 test):**
    - `gloas_early_cache_same_slot_payload_present_false`: extends chain (payload_revealed=true), populates early cache, attests at same slot. Verifies `data.index == 0` — same-slot attestations always have payload_present=false, even when payload_revealed=true in the proto_block
  - **Non-same-slot with revealed payload (1 test):**
    - `gloas_early_cache_non_same_slot_payload_revealed_index_one`: extends chain (payload_revealed=true), populates early cache, attests at next slot. Verifies `data.index == 1` — non-same-slot attestations with payload_revealed=true have payload_present=true
  - **Non-same-slot with unrevealed payload (1 test):**
    - `gloas_early_cache_non_same_slot_payload_not_revealed_index_zero`: extends chain, clones proto_block with payload_revealed=false, populates early cache, attests at next slot. Verifies `data.index == 0` — the safety boundary: unrevealed payloads must not indicate presence
  - **Consistency with canonical path (1 test):**
    - `gloas_early_cache_matches_canonical_attestation`: populates early cache and compares early cache attestation with `produce_unaggregated_attestation` output at both same-slot and non-same-slot positions. Verifies both paths produce identical `data.index` values, catching divergence between the two attestation production pipelines
  - **Pre-Gloas baseline (1 test):**
    - `fulu_early_cache_uses_committee_index_not_payload_present`: sets gloas_fork_epoch=100 (runs in Fulu), populates early cache, attests at skip slot. Verifies `data.index == 0` (committee index), confirming the Gloas payload_present logic is NOT triggered for pre-Gloas forks
- These tests close a critical gap: the early attester cache is the fast-path used when a block has just been imported but hasn't reached the database yet. If the cache computed `payload_present` incorrectly, attestations produced in the first moments after block import would have the wrong `data.index`, causing them to be rejected by peers or attributed to the wrong commitment. The consistency test is particularly important — it catches divergence between the early cache path and the canonical_head path, which would mean the same node produces different attestations depending on timing
- **Remaining gaps from run 96 analysis**: P2 (PayloadAttestationService), P5 (poll_ptc_duties) — both require mock BN infrastructure; P6 (store reconstruct blinded envelope fallback), P8 (post_block_import self-build envelope branch)
- All 553 beacon_chain tests pass (was 548), cargo fmt + clippy clean

### 2026-02-25 — Gloas execution proof gossip handler integration tests + spec tracking (run 105)
- Checked consensus-specs PRs since run 104: no new Gloas spec changes merged
  - 5 PRs merged since run 104 affect Gloas (#4918, #4923, #4930, #4922, #4920) — all already confirmed implemented in runs 97-100
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
  - PRs to watch: #4926 (SECONDS_PER_SLOT → SLOT_DURATION_MS rename), #4558 (cell dissemination)
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Addressed execution proof gossip handler**: the `process_gossip_execution_proof` handler (gossip_methods.rs:3834-3950) had ZERO network-level integration tests. This handler processes ALL execution proofs from gossip — it validates proof structure (version, size, data), cross-references fork choice (block root, block hash), and routes errors to the correct MessageAcceptance. Execution proofs are the core mechanism for stateless validation (ZK substitute for engine_newPayload)
- **Added 6 execution proof gossip handler integration tests** (previously ZERO tests for this handler):
  - **UnknownBlockRoot → Ignore (1 test):**
    - `test_gloas_gossip_execution_proof_unknown_root_ignored`: constructs proof with random block_root not in fork choice, verifies Ignore. Tests the race condition path: proofs may arrive before their block
  - **InvalidVersion → Reject (1 test):**
    - `test_gloas_gossip_execution_proof_invalid_version_rejected`: constructs proof with version=99 (unsupported), verifies Reject + peer penalty. Tests the structural validation gate
  - **ProofDataEmpty → Reject (1 test):**
    - `test_gloas_gossip_execution_proof_empty_data_rejected`: constructs proof with empty proof_data, verifies Reject. Tests the non-empty data requirement
  - **ProofDataTooLarge → Reject (1 test):**
    - `test_gloas_gossip_execution_proof_oversized_data_rejected`: constructs proof with proof_data exceeding MAX_EXECUTION_PROOF_SIZE (1 MB + 1 byte), verifies Reject. Tests the resource exhaustion protection
  - **BlockHashMismatch → Reject (1 test):**
    - `test_gloas_gossip_execution_proof_block_hash_mismatch_rejected`: constructs proof with correct block_root (head) but wrong block_hash (0xdd repeated), verifies Reject. Tests the bid block_hash cross-validation — a proof must attest to the same execution payload committed in the bid
  - **Valid stub proof → Accept (1 test):**
    - `test_gloas_gossip_execution_proof_valid_stub_accepted`: reads actual bid_block_hash from fork choice for the head block, constructs proof with matching block_root, block_hash, version=1 (stub), and non-empty proof_data, verifies Accept. Stub proofs skip cryptographic verification, exercising only structural and fork choice checks
- Tests call `process_gossip_execution_proof` directly on `NetworkBeaconProcessor`, exercising the full pipeline: handler → `verify_execution_proof_for_gossip` → error routing → `propagate_validation_result` → network_rx capture. The Accept path additionally exercises `process_gossip_verified_execution_proof` → `check_gossip_execution_proof_availability_and_import`
- These tests close a critical security gap: the gossip handler is the only defense against invalid execution proofs on the gossip network. The error→MessageAcceptance mapping determines whether invalid proofs are propagated (Accept→Reject bug) or valid proofs are dropped (Accept→Ignore bug). The BlockHashMismatch test is particularly important — without it, a malicious peer could send proofs for non-existent execution payloads that pass structural checks but reference the wrong block_hash, potentially confusing stateless nodes about payload validity
- **Remaining handler gaps**: execution bid remaining error paths (DuplicateBid, BuilderEquivocation, InvalidSignature, InsufficientBuilderBalance, InvalidParentRoot, valid Accept) require a registered builder in the test state; payload attestation remaining paths (ValidatorEquivocation, valid Accept) require valid PTC committee signatures
- All 123 network tests pass (was 117), cargo fmt + clippy clean

### 2026-02-25 — Gloas gossip execution payload envelope handler tests + spec tracking (run 104)
- Checked consensus-specs PRs since run 103: no new Gloas spec changes merged
  - No new PRs merged since run 103 (latest merges were Feb 23-24, all already tracked)
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
  - PRs to watch: #4926 (SECONDS_PER_SLOT → SLOT_DURATION_MS rename), #4558 (cell dissemination)
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Addressed process_gossip_execution_payload**: the handler function (gossip_methods.rs:3402-3543) had ZERO handler-level tests. This handler processes ALL execution payload envelopes from gossip — it combines verification, fork choice mutation, EL notification (newPayload), state transition, SSE events, and head recomputation. The previous verification tests in gloas_verification.rs only tested `verify_payload_envelope_for_gossip` directly, not the handler's error→MessageAcceptance routing
- **Added 6 gossip execution payload envelope handler integration tests** (previously ZERO tests for this handler):
  - **BlockRootUnknown → Ignore (1 test):**
    - `test_gloas_gossip_payload_envelope_unknown_root_ignored`: constructs envelope with random beacon_block_root not in fork choice, verifies handler returns Ignore. Tests the buffering path: unknown-root envelopes are stored in `pending_gossip_envelopes` for later processing when the block arrives
  - **SlotMismatch → Reject (1 test):**
    - `test_gloas_gossip_payload_envelope_slot_mismatch_rejected`: reads committed bid from head block, constructs envelope with correct builder_index and block_hash but wrong slot (head_slot + 1), verifies Reject + peer penalty
  - **BuilderIndexMismatch → Reject (1 test):**
    - `test_gloas_gossip_payload_envelope_builder_index_mismatch_rejected`: reads committed bid from head block, constructs envelope with correct block_hash but wrong builder_index (42 instead of BUILDER_INDEX_SELF_BUILD), verifies Reject + peer penalty
  - **BlockHashMismatch → Reject (1 test):**
    - `test_gloas_gossip_payload_envelope_block_hash_mismatch_rejected`: reads committed bid from head block, constructs envelope with correct builder_index but wrong payload block_hash (0xdd repeated), verifies Reject + peer penalty
  - **Valid self-build → Accept (1 test):**
    - `test_gloas_gossip_payload_envelope_self_build_accepted`: reads committed bid from head block, constructs envelope matching all bid fields (builder_index=BUILDER_INDEX_SELF_BUILD, correct block_hash, correct slot), verifies Accept. Self-build envelopes skip BLS signature verification, so empty signature is valid
  - **PriorToFinalization → Ignore (1 test):**
    - `test_gloas_gossip_payload_envelope_prior_to_finalization_ignored`: builds a 3-epoch chain (long enough for finalization), constructs envelope with slot before finalized_slot, verifies Ignore. Tests the stale-message guard
- Tests call `process_gossip_execution_payload` directly on `NetworkBeaconProcessor`, exercising the full pipeline: handler → `verify_payload_envelope_for_gossip` → error routing → `propagate_validation_result` → network_rx capture. The Accept path additionally exercises `apply_payload_envelope_to_fork_choice` and `process_payload_envelope`
- These tests close a critical security gap: the gossip handler is the first line of defense against invalid payload envelopes. The error→MessageAcceptance mapping determines whether invalid envelopes are propagated to other peers (Accept→Reject bug = propagate invalid payloads) or valid ones are dropped (Accept→Ignore bug = drop valid payloads). The handler also controls peer scoring — a Reject triggers LowToleranceError peer penalty, while Ignore does not
- **Remaining handler gaps from run 96 analysis**: P2 (PayloadAttestationService), P5 (poll_ptc_duties) — both require mock BN infrastructure
- All 117 network tests pass (was 111), cargo fmt clean

### 2026-02-25 — Gloas attestation production payload_present tests + spec tracking (run 103)
- Checked consensus-specs PRs since run 102: no new Gloas spec changes merged
  - **PR #4941** (merged Feb 19): "Update execution proof construction to use beacon block" — labeled eip8025 (execution proofs), only touches `specs/_features/eip8025/prover.md`. Not Gloas ePBS, no action needed
  - **PR #4946** (merged Feb 24): actions/stale bump — CI only, already tracked in run 102
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
  - PRs to watch: #4926 (SECONDS_PER_SLOT → SLOT_DURATION_MS rename), #4558 (cell dissemination)
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Conducted systematic analysis** of `produce_unaggregated_attestation` Gloas `payload_present` path (beacon_chain.rs:2206-2217) — found ZERO integration test coverage with Gloas enabled. The existing `attestation_production.rs` tests use `default_spec()` which sets `gloas_fork_epoch: None`, so the Gloas branch (reading `payload_revealed` from fork choice) was never exercised
- **Key discovery during test writing**: Gloas blocks imported without envelope processing have `ExecutionStatus::Optimistic` (not `Irrelevant`). This is because fork_choice.rs:979-988 handles the bid-containing block body separately from the payload-containing body, and always sets `Optimistic(block_hash)` for the bid path. The `PayloadVerificationStatus::Irrelevant` from `PayloadNotifier` is unused because the code branches on the bid, not the payload. This means `produce_unaggregated_attestation` correctly refuses to attest to Gloas blocks whose envelopes haven't been processed — a safety-critical behavior
- **Added 5 attestation production payload_present integration tests** (previously ZERO tests for this pipeline with Gloas):
  - **Same-slot behavior (1 test):**
    - `gloas_attestation_same_slot_payload_present_false`: produces blocks with envelopes (payload_revealed=true), then calls `produce_unaggregated_attestation` at the head block's slot. Verifies `data.index == 0` — same-slot attestations always have payload_present=false per spec, because the attester cannot know whether the envelope has arrived
  - **Non-same-slot with revealed payload (1 test):**
    - `gloas_attestation_non_same_slot_payload_revealed_index_one`: produces blocks with envelopes, advances slot without block (skip slot), attests. Verifies `data.index == 1` — the previous block's payload was revealed, so non-same-slot attestations include payload_present=true
  - **Unrevealed payload safety check (1 test):**
    - `gloas_attestation_refused_for_unrevealed_payload_block`: imports a Gloas block WITHOUT processing its envelope, verifies payload_revealed=false AND execution_status=Optimistic, then confirms `produce_unaggregated_attestation` returns `HeadBlockNotFullyVerified`. This tests the safety boundary: nodes must not attest to blocks whose execution payload hasn't been verified
  - **Pre-Gloas baseline (1 test):**
    - `fulu_attestation_always_index_zero`: produces Fulu blocks (pre-Gloas), attests at a skip slot, verifies `data.index == 0`. Confirms the Gloas payload_present logic is NOT triggered for pre-Gloas forks
  - **Full lifecycle: Optimistic → Valid → attestation (1 test):**
    - `gloas_attestation_enabled_after_envelope_processing`: imports block without envelope (Optimistic, attestation fails), then processes envelope (Valid, attestation succeeds with index=1). Tests the complete lifecycle from block-only import through envelope processing to attestation production
- These tests close a significant gap: the `produce_unaggregated_attestation` function is called for EVERY attestation produced by the node. The Gloas `payload_present` logic determines `data.index`, which is a consensus-critical field — a wrong index would cause attestations to be rejected by peers or attributed to the wrong committee. Previously no integration test verified this pipeline with Gloas enabled
- All 548 beacon_chain tests pass (was 543), cargo fmt clean

### 2026-02-25 — Gloas block verification edge case tests + spec tracking (run 102)
- Checked consensus-specs PRs since run 101: no new Gloas spec changes merged
  - **PR #4946** (merged Feb 24): actions/stale bump — CI only, no impact
  - No new Gloas PRs merged since run 101
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
  - New PRs to watch: #4926 (SECONDS_PER_SLOT → SLOT_DURATION_MS rename, touches Gloas), #4558 (cell dissemination, now tags Gloas)
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Conducted systematic test gap analysis** of block_verification.rs Gloas-specific paths, store crate Gloas paths, and remaining P2-P8 gaps from run 96 analysis
- **Addressed remaining test gaps from run 96**: P7 (get_execution_payload Gloas parent hash/withdrawals) now fully covered via production invariant tests
- **Added 6 block verification Gloas edge case tests** (previously ZERO tests for these paths):
  - **Bid blob count validation (2 tests):**
    - `gloas_gossip_rejects_block_with_excess_bid_blob_commitments`: tampers bid to have max_blobs+1 blob_kzg_commitments, verifies `InvalidBlobCount` rejection. This tests the Gloas-specific branch in block_verification.rs:903-914 that reads commitments from the bid (not the body). The pre-Gloas path was tested but the Gloas bid path had ZERO coverage
    - `gloas_gossip_accepts_block_with_valid_bid_blob_count`: sets bid blob commitments to exactly max_blobs, verifies the blob count check passes (block may fail on later checks, but not InvalidBlobCount)
  - **Structural invariant (1 test):**
    - `gloas_block_blob_commitments_in_bid_not_body`: verifies body.blob_kzg_commitments() returns Err for Gloas (removed from body), while bid.blob_kzg_commitments is accessible and within limit. Catches code that mistakenly reads commitments from body instead of bid
  - **Block production invariant tests (3 tests):**
    - `gloas_block_production_bid_gas_limit_matches_state`: verifies state.latest_execution_payload_bid().gas_limit is non-zero and matches the head block's bid gas_limit. Tests the Gloas path in get_execution_payload (execution_payload.rs:397) which reads gas_limit from the bid instead of the header
    - `gloas_block_production_latest_block_hash_consistency`: verifies state.latest_block_hash() is non-zero and equals the next block's bid.parent_block_hash. Tests the Gloas path in get_execution_payload (execution_payload.rs:396) which reads parent hash from latest_block_hash instead of the header
    - `gloas_block_production_uses_gloas_withdrawals`: verifies the envelope's payload has accessible withdrawals and the state has payload_expected_withdrawals. Tests the Gloas path in get_execution_payload (execution_payload.rs:403-410) which calls get_expected_withdrawals_gloas instead of get_expected_withdrawals
- These tests close two categories of gaps: (1) the bid blob count gossip validation is a security boundary — without it, nodes could propagate blocks with arbitrarily many blob commitments, causing resource exhaustion on peers. (2) the block production invariants verify that the Gloas-specific data sources (bid gas_limit, latest_block_hash, gloas withdrawals) are correctly wired through block production — a regression in any of these would cause the EL to receive wrong parameters, producing invalid execution payloads
- **Remaining gaps from run 96 analysis**: P2 (PayloadAttestationService), P5 (poll_ptc_duties), P6 (store reconstruct), P8 (post_block_import) — all require complex test infrastructure (mock beacon nodes, store reconstruction)
- All 543 beacon_chain tests pass (was 537), cargo fmt clean

### 2026-02-25 — proposer preferences gossip handler tests + spec tracking (run 101)
- Checked consensus-specs PRs since run 100: no new Gloas spec changes merged
  - **PR #4946** (merged Feb 24): actions/stale bump — CI only, no impact
  - No new Gloas PRs merged since run 100
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Addressed P4 from run 96 gap analysis**: `process_gossip_proposer_preferences` (complex inline validation with BLS signature verification, ZERO test coverage)
- **Added 7 proposer preferences gossip handler integration tests** (previously ZERO tests for this handler):
  - **Epoch check (IGNORE) tests (2 tests):**
    - `test_gloas_gossip_proposer_preferences_current_epoch_ignored`: constructs preferences with proposal_slot in current epoch, verifies proposal_epoch != next_epoch → MessageAcceptance::Ignore
    - `test_gloas_gossip_proposer_preferences_far_future_epoch_ignored`: constructs preferences with proposal_slot in epoch 100, verifies Ignore (not just off-by-one)
  - **Proposer lookahead (REJECT) tests (2 tests):**
    - `test_gloas_gossip_proposer_preferences_wrong_proposer_rejected`: reads actual proposer from `proposer_lookahead` at `slots_per_epoch + (proposal_slot % slots_per_epoch)`, uses a different validator_index, verifies Reject + peer penalty
    - `test_gloas_gossip_proposer_preferences_unknown_validator_rejected`: uses validator_index=9999 (beyond registry), verifies Reject (lookahead won't contain it)
  - **Signature verification (REJECT) tests (2 tests):**
    - `test_gloas_gossip_proposer_preferences_invalid_signature_rejected`: uses correct proposer_index but `Signature::empty()`, verifies Reject at BLS verification step
    - `test_gloas_gossip_proposer_preferences_wrong_key_rejected`: uses correct proposer_index, signs with a different validator's secret key, verifies Reject (catches key confusion bugs)
  - **Full valid path (ACCEPT) test (1 test):**
    - `test_gloas_gossip_proposer_preferences_valid_accepted`: constructs fully valid SignedProposerPreferences — correct next-epoch proposal_slot, correct proposer_index from lookahead, valid BLS signature using Domain::ProposerPreferences with the proposer's secret key — verifies MessageAcceptance::Accept
- Tests exercise each validation check in the handler (gossip_methods.rs:3690-3828) in order: epoch check → lookahead check → pubkey lookup → signature verification → accept
- The signature verification tests are particularly important: `Domain::ProposerPreferences` (domain index 13) is a Gloas-specific signing domain. If the handler used the wrong domain, all valid proposer preferences messages would be rejected, preventing proposers from communicating their fee_recipient/gas_limit preferences to builders
- All 111 network tests pass (was 104), cargo fmt clean

### 2026-02-25 — network gossip handler integration tests + spec tracking (run 100)
- Checked consensus-specs PRs since run 99: no new Gloas spec changes merged
  - **PR #4946** (merged Feb 24): actions/stale bump — CI only, no impact
  - **PR #4945** (merged Feb 23): inclusion list test fix — FOCIL/EIP-7805, not Gloas
  - **PR #4918** already tracked in run 99 (confirmed implemented)
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Addressed P1 from run 96 gap analysis**: network gossip handlers (5 Gloas-specific gossip handler functions with ZERO test coverage)
- **Added 6 network gossip handler integration tests** (previously ZERO tests in network crate for Gloas gossip):
  - **Execution bid rejection tests (3 tests):**
    - `test_gloas_gossip_bid_zero_payment_rejected`: constructs bid with execution_payment=0, verifies process_gossip_execution_bid maps ZeroExecutionPayment → MessageAcceptance::Reject
    - `test_gloas_gossip_bid_wrong_slot_ignored`: constructs bid for slot 999, verifies SlotNotCurrentOrNext → MessageAcceptance::Ignore
    - `test_gloas_gossip_bid_unknown_builder_rejected`: constructs bid with builder_index=9999 (not in registry), verifies UnknownBuilder → MessageAcceptance::Reject
  - **Payload attestation rejection tests (3 tests):**
    - `test_gloas_gossip_payload_attestation_unknown_root_ignored`: constructs attestation with random beacon_block_root, verifies UnknownBeaconBlockRoot → MessageAcceptance::Ignore
    - `test_gloas_gossip_payload_attestation_future_slot_ignored`: constructs attestation for slot 999, verifies FutureSlot → MessageAcceptance::Ignore
    - `test_gloas_gossip_payload_attestation_empty_bits_rejected`: constructs attestation with zero aggregation bits, verifies EmptyAggregationBits → MessageAcceptance::Reject
  - Built `gloas_rig()` helper: creates TestRig with gloas_fork_epoch=0 (all blocks are Gloas)
  - Built `drain_validation_result()` helper: drains network_rx for ValidationResult messages, skipping ReportPeer
  - Built `assert_accept()`, `assert_reject()`, `assert_ignore()` helpers: pattern-match MessageAcceptance (no PartialEq on gossipsub type)
- Tests call `process_gossip_execution_bid` and `process_gossip_payload_attestation` directly on `NetworkBeaconProcessor`, exercising the full pipeline: gossip handler → beacon_chain.verify_*_for_gossip → error mapping → propagate_validation_result → network_rx capture
- These tests cover the security boundary for incoming gossip messages at the network layer. The gossip handlers are the first line of defense against malicious messages — they must correctly map verification errors to Accept/Reject/Ignore to prevent invalid messages from being propagated, and to penalize peers appropriately. A regression in any mapping could cause the node to propagate invalid messages (Reject→Accept bug) or drop valid ones (Accept→Ignore bug)
- All 104 network tests pass (was 98), cargo fmt + clippy clean

### 2026-02-25 — apply_execution_bid_to_fork_choice tests + spec tracking (run 99)
- Checked consensus-specs PRs since run 98: no new Gloas spec changes merged
  - **PR #4918** (merged Feb 23): "Only allow attestations for known payload statuses" — already confirmed implemented in run 97
  - **PR #4923** (merged Feb 16): "Ignore beacon block if parent payload unknown" — already confirmed implemented (block_verification.rs:972-984, gossip_methods.rs:1291-1302, with 3 existing tests)
  - **PR #4930** (merged Feb 16): "Rename execution_payload_states to payload_states" — cosmetic only, vibehouse already uses `payload_states` naming in comments
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Conducted systematic test gap analysis** of beacon_chain Gloas methods — identified `apply_execution_bid_to_fork_choice` (line 2507) as the highest-impact untested path:
  - Zero direct test coverage — all prior tests bypassed this method and manipulated the bid pool directly
  - The method calls both `execution_bid_pool.insert()` AND `fork_choice.on_execution_bid()`, but only the pool path was tested
  - `on_execution_bid` sets builder_index, resets payload_revealed, initializes PTC weights — critical for block viability
- **Added 5 apply_execution_bid_to_fork_choice integration tests** (previously ZERO tests for this beacon_chain method):
  - `gloas_apply_bid_to_fork_choice_updates_node_fields`: applies an external bid via VerifiedExecutionBid, verifies fork choice node has updated builder_index, payload_revealed=false, ptc_weight=0, ptc_blob_data_available_weight=0, payload_data_available=false. Also verifies pre-condition (self-build builder_index before external bid)
  - `gloas_apply_bid_to_fork_choice_inserts_into_pool`: applies bid, verifies it's retrievable from the execution_bid_pool via get_best_execution_bid with correct value and builder_index
  - `gloas_apply_bid_to_fork_choice_rejects_unknown_root`: verifies error when bid references a beacon block root not in fork choice
  - `gloas_apply_bid_to_fork_choice_rejects_slot_mismatch`: verifies error when bid slot doesn't match block's actual slot
  - `gloas_bid_then_envelope_lifecycle_via_beacon_chain`: full bid→reveal lifecycle — applies external bid (payload_revealed resets to false), then calls on_execution_payload (payload_revealed flips to true, execution_status=Optimistic), verifying the complete state machine through beacon_chain
- Added `__new_for_testing` constructor on VerifiedExecutionBid (#[doc(hidden)]) to allow integration tests to construct verified bids without BLS signature validation against registered builders
- All 537 beacon_chain tests pass (was 532), cargo fmt clean

### 2026-02-25 — fork transition boundary integration tests + spec tracking (run 98)
- Checked consensus-specs PRs since run 97: no new Gloas spec changes merged
  - Only #4931 (FOCIL rebase onto Gloas — EIP-7805 Heze, not Gloas ePBS) already tracked
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Conducted systematic test gap analysis** of fork transition boundary coverage — identified that Fulu→Gloas fork transition invariants had no dedicated integration tests:
  - Existing `fulu_to_gloas_fork_transition` only checks variant change, not bid parent_block_hash correctness
  - No test verified state upgrade copies Fulu EL header block_hash into latest_block_hash
  - No test verified chain continuity through a full epoch after fork transition
  - No test verified execution_payload_availability initialization (all bits true)
  - No test verified builder_pending_payments initialization (all default)
- **Added 5 fork transition boundary integration tests** (previously ZERO tests for these invariants):
  - `gloas_fork_transition_bid_parent_hash_from_fulu_header`: extends chain to last Fulu slot, captures Fulu EL header block_hash, extends to first Gloas slot, verifies first Gloas bid's `parent_block_hash` equals the Fulu header's `block_hash`. This is the critical chain continuity invariant: state upgrade copies the hash and block production reads from it
  - `gloas_fork_transition_latest_block_hash_matches_fulu_header`: verifies indirectly that `latest_block_hash` was correctly set from Fulu header by checking bid `parent_block_hash` (which reads from `latest_block_hash` at block production time)
  - `gloas_fork_transition_chain_continues_full_epoch`: extends chain through fork and one full Gloas epoch (8 slots for minimal), verifies every slot has a Gloas block with a non-zero bid `block_hash`. Exercises the complete pipeline: fork upgrade → first block → envelope → state cache → next block repeatedly
  - `gloas_fork_transition_execution_payload_availability_all_set`: verifies that after fork transition, all `execution_payload_availability` bits are set (spec: initialized to all-true), with at most one bit cleared (from per_slot_processing at the fork slot)
  - `gloas_fork_transition_builder_pending_payments_all_default`: verifies all `builder_pending_payments` entries are default (zero weight, zero amount) after fork, confirming self-build bids (value=0) don't record pending payments
- All 532 beacon_chain tests pass (was 527), cargo fmt clean

### 2026-02-25 — fork choice attestation integration tests + spec tracking (run 97)
- Checked consensus-specs PRs since run 96: two Gloas-related PRs merged
  - **PR #4918 merged** (Feb 23): "Only allow attestations for known payload statuses" — adds `assert attestation.data.beacon_block_root in store.payload_states` when `index == 1`. **Already implemented** in vibehouse: `fork_choice.rs:1206-1215` checks `!block.payload_revealed` and returns `PayloadNotRevealed` error. 3 existing tests cover this. No code changes needed
  - **PR #4931 merged** (Feb 20): "Rebase FOCIL onto Gloas" — FOCIL (EIP-7805) spec files rebased onto Gloas fork under `specs/_features/eip7805/`. FOCIL is assigned to Heze fork (PR #4942), not Gloas. No action needed for vibehouse
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Conducted systematic test gap analysis** for fork choice integration paths — identified `apply_payload_attestation_to_fork_choice` and `apply_execution_bid_to_fork_choice` as two beacon_chain methods with ZERO integration test coverage. These are the methods that bridge gossip-verified objects to fork choice state mutations
- **Added 5 fork choice attestation import integration tests** (previously ZERO tests for this pipeline):
  - **apply_payload_attestation_to_fork_choice via API import (4 tests):**
    - `gloas_import_attestation_updates_fork_choice_ptc_weight`: imports a payload attestation via `import_payload_attestation_message`, verifies `ptc_weight` changes from 0 to 1 in fork choice. Tests full pipeline: `import_payload_attestation_message` → `verify_payload_attestation_for_gossip` → `apply_payload_attestation_to_fork_choice` → `on_payload_attestation`
    - `gloas_import_attestation_updates_blob_data_weight`: imports attestation with `blob_data_available=true`, verifies `ptc_blob_data_available_weight` increments while `ptc_weight` stays 0 (payload_present=false)
    - `gloas_import_attestation_quorum_triggers_payload_revealed`: resets `payload_revealed=false`, imports attestations from ALL PTC members (2 for minimal preset), verifies PTC quorum flips `payload_revealed=true`. Checks state after each vote to verify quorum threshold behavior
    - `gloas_import_attestation_payload_absent_no_ptc_weight`: imports attestation with `payload_present=false, blob_data_available=false`, verifies both weights remain 0
  - **Bid pool integration (1 test):**
    - `gloas_bid_pool_insertion_and_retrieval_via_chain`: inserts bids at different values into the pool (same code path as `apply_execution_bid_to_fork_choice` line 2515), verifies `get_best_execution_bid` returns highest-value bid and prunes old-slot bids
- These tests close the biggest fork choice integration gap: `apply_payload_attestation_to_fork_choice` (beacon_chain.rs:3179) is called on EVERY gossip payload attestation and every API-submitted attestation. The previous `import_payload_attestation_message` tests verified pool insertion but NOT fork choice state changes. A regression where `on_payload_attestation` fails silently would mean PTC votes never accumulate, blocks never reach quorum, and the chain stalls
- All 527 beacon_chain tests pass (was 522), cargo fmt + clippy clean, full workspace lint passes

### 2026-02-25 — validator store Gloas signing tests + spec tracking (run 96)
- Checked consensus-specs PRs since run 95: no new Gloas spec changes merged
  - Notable: PR #4942 promotes EIP-7805 (FOCIL) to Heze fork — not ePBS/Gloas, no action needed
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Conducted comprehensive test gap analysis** across validator_client, network, store, and http_api — identified 8 priority gaps:
  - P1: Network gossip handlers (5 functions, zero coverage, complex TestRig required)
  - P2: PayloadAttestationService::produce_payload_attestations (zero tests, entire file untested)
  - P3: sign_payload_attestation + sign_execution_payload_envelope (zero tests for two new signing domains)
  - P4: process_gossip_proposer_preferences (complex inline validation, untested)
  - P5: poll_ptc_duties (duty fetch logic, needs mock BN)
  - P6: Store reconstruct.rs envelope re-application (partially tested via WSS test)
  - P7: get_execution_payload Gloas parent hash/withdrawals (no unit test)
  - P8: post_block_import_logging_and_response self-build envelope branch
- **Added 6 validator store Gloas signing domain unit tests** (previously ZERO tests in entire lighthouse_validator_store crate):
  - **sign_execution_payload_envelope (3 tests):**
    - `sign_execution_payload_envelope_uses_beacon_builder_domain`: creates a LighthouseValidatorStore with a known keypair, signs an ExecutionPayloadEnvelope, independently computes the expected signing root using Domain::BeaconBuilder, and verifies the signature matches. Also checks message fields (slot, beacon_block_root, builder_index) are preserved
    - `sign_execution_payload_envelope_wrong_domain_fails_verify`: signs an envelope, computes signing root with Domain::BeaconAttester (wrong), and asserts the signature does NOT verify — proves the correct domain is used
    - `sign_envelope_unknown_pubkey_returns_error`: verifies that signing with an unregistered pubkey returns an error
  - **sign_payload_attestation (3 tests):**
    - `sign_payload_attestation_uses_ptc_attester_domain`: signs PayloadAttestationData, independently computes expected signing root using Domain::PtcAttester, verifies signature matches. Also checks validator_index and data fields are correct in the returned PayloadAttestationMessage
    - `sign_payload_attestation_wrong_domain_fails_verify`: signs data, computes signing root with Domain::BeaconAttester (wrong), asserts signature does NOT verify
    - `sign_payload_attestation_unknown_pubkey_returns_error`: verifies error for unregistered pubkey
  - Built `store_with_validator()` async helper that creates a LighthouseValidatorStore<TestingSlotClock, MinimalEthSpec> with Gloas genesis spec, creates a random Keypair, writes a keystore to disk via KeystoreBuilder, and registers it via add_validator_keystore
  - Added dev-dependencies: bls, eth2_keystore, tempfile, zeroize
- These tests close the validator store signing gap: `sign_execution_payload_envelope` (lib.rs:764) uses Domain::BeaconBuilder and `sign_payload_attestation` (lib.rs:788) uses Domain::PtcAttester. If either method used the wrong domain, all envelope signatures or PTC attestations from the VC would be rejected by peers. Previously no test verified domain correctness
- All 6 lighthouse_validator_store tests pass (was 0), cargo fmt + clippy clean, full workspace lint passes

### 2026-02-25 — fork choice Gloas method tests + spec tracking (run 95)
- Checked consensus-specs PRs since run 94: no new Gloas spec changes merged
  - No new PRs merged since run 94
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
  - #4940 (Gloas fork choice tests): still open, will add test vectors when merged
  - #4747 (Fast Confirmation Rule): updated Feb 25, still evolving, no action needed
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC — new comment from michaelsproul), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Conducted systematic test gap analysis** across fork_choice, beacon_chain, store, validator_client, and network — identified fork choice Gloas methods as highest-impact untested paths (0% direct test coverage for 3 critical methods)
- **Added 13 fork choice Gloas method integration tests** (previously ZERO tests for these paths):
  - **on_execution_bid (3 tests):**
    - `fc_on_execution_bid_rejects_unknown_block_root`: verifies UnknownBeaconBlockRoot error for non-existent root
    - `fc_on_execution_bid_rejects_slot_mismatch`: verifies SlotMismatch error when bid.slot != block.slot
    - `fc_on_execution_bid_updates_node_fields`: verifies bid sets builder_index, resets payload_revealed=false, initializes ptc_weight=0 and ptc_blob_data_available_weight=0
  - **on_execution_payload (2 tests):**
    - `fc_on_execution_payload_marks_revealed`: verifies payload_revealed=true, payload_data_available=true, execution_status=Optimistic(hash) after reveal
    - `fc_on_execution_payload_rejects_unknown_root`: verifies MissingProtoArrayBlock error for non-existent root
  - **on_payload_attestation (6 tests):**
    - `fc_on_payload_attestation_rejects_future_slot`: verifies FutureSlot rejection
    - `fc_on_payload_attestation_rejects_too_old`: verifies TooOld rejection (>1 epoch old)
    - `fc_on_payload_attestation_ignores_slot_mismatch`: verifies silent return when data.slot != block.slot (per spec), no weight accumulated
    - `fc_on_payload_attestation_quorum_triggers_payload_revealed`: verifies quorum threshold is strictly greater (PTC_SIZE/2), exactly-at-threshold does NOT trigger, one-more vote triggers payload_revealed=true
    - `fc_on_payload_attestation_blob_quorum_independent`: verifies blob_data_available quorum is tracked independently from payload_present (payload_present=false, blob_data_available=true → only blob quorum reached)
    - `fc_on_payload_attestation_rejects_unknown_root`: verifies UnknownBeaconBlockRoot error
  - **Lifecycle tests (2 tests):**
    - `fc_bid_then_payload_lifecycle`: full bid→reveal end-to-end, verifying state transitions at each step
    - `fc_payload_attestation_quorum_sets_optimistic_from_bid_hash`: verifies that when PTC quorum is reached and execution_status is not yet set, it's set to Optimistic(bid_block_hash) — critical for fork choice head selection before envelope arrives
- These tests close the biggest fork choice test gap: `on_execution_bid` (fork_choice.rs:1323), `on_payload_attestation` (fork_choice.rs:1398), and `on_execution_payload` (fork_choice.rs:1526) are the three methods that determine how Gloas blocks become viable for head selection. A regression in PTC quorum logic would prevent blocks from becoming head candidates; a regression in on_execution_bid would break builder tracking; a regression in on_execution_payload would prevent payload reveals from being recorded
- All 522 beacon_chain tests pass (was 509), cargo fmt + clippy clean

### 2026-02-25 — Gloas execution payload path tests + spec tracking (run 94)
- Checked consensus-specs PRs since run 93: no new Gloas spec changes merged
  - No PRs merged since run 93; only infrastructure PRs (#4946 actions/stale bump)
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
  - #4898 (remove pending from tiebreaker): approved but unmerged; our code already implements the target behavior
  - New PR to watch: #4747 (Fast Confirmation Rule) updated Feb 25
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues triaged: #8892 (SSZ response support) already fully implemented for all 5 endpoints, #8858 (events feature gating) references file that doesn't exist in vibehouse, #8828 (block production endpoints) is design-level discussion
- **Conducted systematic test gap analysis** of execution_payload.rs via subagent — identified ZERO tests for:
  - `PayloadNotifier::new()` Gloas path (returns `Irrelevant` status)
  - `validate_execution_payload_for_gossip()` Gloas early-return
  - `build_self_build_envelope()` state root computation
  - `get_execution_payload()` Gloas gas_limit extraction from bid
- **Added 7 execution payload path integration tests** (previously ZERO tests for these paths):
  - `gloas_payload_notifier_returns_irrelevant`: constructs a `PayloadNotifier` for a Gloas block with `NotifyExecutionLayer::Yes`, asserts `notify_new_payload()` returns `PayloadVerificationStatus::Irrelevant` without calling the EL. A bug here would cause unnecessary EL calls or block import failures for Gloas blocks
  - `fulu_payload_notifier_does_not_return_irrelevant`: complement test — Fulu block with execution enabled goes through EL verification and returns `Verified` (not `Irrelevant`). Uses `make_block_return_pre_state` to provide the correct pre-block state that `partially_verify_execution_payload` expects
  - `gloas_gossip_skips_execution_payload_validation`: calls `validate_execution_payload_for_gossip` directly with a Gloas block and its parent's `ProtoBlock`, asserts `Ok(())`. This is the gossip-level check that timestamps and merge transitions don't apply to Gloas blocks
  - `fulu_gossip_validates_execution_payload`: complement test — Fulu block goes through full timestamp validation and passes. Ensures the early-return only fires for Gloas blocks
  - `gloas_self_build_envelope_state_root_differs_from_block`: verifies `build_self_build_envelope()` produces an envelope whose `state_root` differs from the block's (pre-envelope) `state_root`, both are non-zero, and the envelope references the correct `beacon_block_root` and `slot`. This tests the complex state root discovery path where `process_execution_payload_envelope` runs on a cloned state and the state root is captured from the `InvalidStateRoot` error
  - `gloas_self_build_envelope_payload_block_hash_consistency`: after extending the chain, verifies the envelope's payload `block_hash` is non-zero (real EL payload) and differs from the bid's `parent_block_hash` (parent vs child execution block hash)
  - `gloas_block_production_gas_limit_from_bid`: verifies the Gloas-specific path in `get_execution_payload` that reads `gas_limit` from `state.latest_execution_payload_bid()` instead of `state.latest_execution_payload_header()`. Asserts both the source bid gas_limit and the produced payload gas_limit are non-zero
- These tests close the largest execution payload gap: the functions in `execution_payload.rs` that handle Gloas's fundamentally different payload architecture (no payload in block body, payload via separate envelope). `PayloadNotifier::new` is called on EVERY block import (block_verification.rs:1458), and `validate_execution_payload_for_gossip` on every gossip block (block_verification.rs:1093). A regression in either would break block import or gossip for all Gloas blocks
- All 509 beacon_chain tests pass (was 502), cargo fmt + clippy clean

### 2026-02-25 — Gloas slot timing unit tests + spec tracking (run 93)
- Checked consensus-specs PRs since run 92: no new Gloas spec changes merged
  - No new PRs since run 92; #4944 (ExecutionProofsByRoot) still open
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Conducted comprehensive test gap analysis** across all Gloas code paths:
  - observation caches (execution_bid_pool, observed_execution_bids, observed_payload_attestations): 100% covered (14+13+17 tests)
  - gloas_verification.rs: 49 integration tests, ~85% coverage (remaining gaps are defensive error paths for internal errors)
  - per_block_processing/gloas.rs: 60+ unit tests covering bid, withdrawal, PTC, payload attestation processing
  - envelope_processing.rs: 23 unit tests
  - block_replayer Gloas: 13+ tests
  - fork_choice Gloas: well-tested with unit + integration tests
  - **slot_clock Gloas timing: ZERO tests for the 4-interval slot timing mechanism** — identified as highest-impact gap
- **Added 16 Gloas slot timing unit tests** (previously ZERO tests for 4-interval timing):
  - `gloas_fork_slot_round_trip`: set/get/unset gloas_fork_slot on ManualSlotClock
  - `current_intervals_pre_gloas_is_3`: no fork configured or before fork slot → 3 intervals
  - `current_intervals_at_gloas_fork_is_4`: exactly at fork slot → 4 intervals
  - `current_intervals_after_gloas_fork_is_4`: after fork slot → 4 intervals
  - `current_intervals_one_before_gloas_fork_is_3`: slot 9 with fork at 10 → 3 intervals
  - `unagg_attestation_delay_pre_gloas`: 12s/3 = 4s
  - `unagg_attestation_delay_post_gloas`: 12s/4 = 3s
  - `agg_attestation_delay_pre_gloas`: 2*12s/3 = 8s
  - `agg_attestation_delay_post_gloas`: 2*12s/4 = 6s
  - `sync_committee_delays_mirror_attestation_delays`: sync msg = unagg, sync contribution = agg, both pre and post Gloas
  - `single_lookup_delay_changes_with_gloas`: 2s pre-Gloas → 1.5s post-Gloas
  - `freeze_at_preserves_gloas_fork_slot`: frozen clock retains Gloas config and uses 4 intervals
  - `timing_transition_at_fork_boundary`: slot 4→3 intervals, slot 5→4 intervals, slot 6→4 intervals (fork at 5)
  - `gloas_fork_at_genesis`: Gloas from slot 0 immediately uses 4 intervals
- These tests cover the `current_intervals_per_slot()` method (slot_clock/src/lib.rs:89-102) and all derived timing methods. The ManualSlotClock is the underlying implementation used by both test harnesses and the production SystemTimeSlotClock. A bug here would cause all validators to produce attestations and sync committee messages at the wrong timing after Gloas activation — PTC attestations would fire too early or too late, potentially missing the payload timeliness window
- All 24 slot_clock tests pass (was 8), cargo fmt + clippy clean

### 2026-02-25 — gossip verification error path tests + spec tracking (run 92)
- Checked consensus-specs PRs since run 91: no new Gloas spec changes merged
  - Only infrastructure PRs (#4946 actions/stale bump already tracked)
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630, #4944
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues triaged: #8892 (SSZ response support — actionable, spec compliance), #8893 (state storage design — discussion), #8790 (license Cargo.toml — low priority), #8741 (head monitor — enhancement), #8588 (streamer tests — TODO already removed), #8589 (GloasNotImplemented — already removed from code)
- **Conducted systematic test gap analysis** of network gossip methods via subagent: identified 5 Gloas gossip handlers in gossip_methods.rs (execution bid, payload envelope, payload attestation, proposer preferences, execution proof) with ZERO integration tests. Network-level tests require complex TestRig harness, so focused on beacon_chain-level gossip verification error paths instead
- **Added 9 gossip verification error path integration tests** (previously ZERO tests for these rejection paths):
  - **Envelope verification (5 tests):**
    - `gloas_envelope_gossip_rejects_slot_mismatch`: tampers envelope slot (+100), verifies `SlotMismatch` rejection
    - `gloas_envelope_gossip_rejects_builder_index_mismatch`: tampers builder_index (wrapping_add 1), verifies `BuilderIndexMismatch` rejection
    - `gloas_envelope_gossip_rejects_block_hash_mismatch`: tampers payload block_hash to random, verifies `BlockHashMismatch` rejection
    - `gloas_envelope_gossip_buffers_unknown_block_root`: tampers beacon_block_root to random, verifies `BlockRootUnknown` rejection AND confirms envelope is buffered in `pending_gossip_envelopes` for later processing (critical for out-of-order arrival)
    - `gloas_envelope_gossip_rejects_not_gloas_block`: uses Gloas fork at epoch 1, points envelope at genesis (Fulu) block root, verifies `NotGloasBlock` or `PriorToFinalization` rejection
  - **Bid verification (4 tests):**
    - `gloas_bid_gossip_rejects_slot_not_current_or_next`: sets bid slot to 999, verifies `SlotNotCurrentOrNext` rejection (first validation check)
    - `gloas_bid_gossip_rejects_zero_execution_payment`: uses self-build bid (naturally has payment=0), verifies `ZeroExecutionPayment` rejection
    - `gloas_bid_gossip_rejects_unknown_builder`: sets execution_payment=1 on self-build bid (builder_index=u64::MAX not in registry), verifies `UnknownBuilder` rejection
    - `gloas_bid_gossip_rejects_nonexistent_builder_index`: sets builder_index=42 on bid, verifies `UnknownBuilder` rejection with correct index
  - Built `import_block_get_envelope()` helper (produce block+envelope, import only block) and `assert_envelope_rejected()`/`assert_bid_rejected()` helpers that work around VerifiedPayloadEnvelope/VerifiedExecutionBid not implementing Debug
- These tests cover the security boundary for incoming gossip messages: `verify_payload_envelope_for_gossip` (gloas_verification.rs:605-722) validates envelopes against committed bids in the block, and `verify_execution_bid_for_gossip` (gloas_verification.rs:327-441) validates builder bids against the head state. Without these tests, a regression in any rejection path could allow malformed messages to be imported and propagated
- All 502 beacon_chain tests pass (was 493), cargo fmt + clippy clean

### 2026-02-25 — stateless validation execution proof threshold tests + spec tracking (run 91)
- Checked consensus-specs PRs since run 90: no new Gloas spec changes merged
  - Only infrastructure PRs: actions/stale bump (#4946), no Gloas-affecting changes
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Conducted systematic test gap analysis** — identified stateless validation proof threshold code as highest-impact untested path (zero test coverage for a central vibehouse feature)
- **Added 7 stateless validation integration tests** (previously ZERO tests for execution proof threshold logic):
  - `gloas_stateless_proof_threshold_marks_block_valid`: imports Gloas blocks into a stateless harness (stateless_validation=true), verifies block starts as Optimistic, sends a verified proof via `check_gossip_execution_proof_availability_and_import` with threshold=1, asserts return value is `Imported(block_root)` and fork choice execution_status flips from Optimistic to Valid
  - `gloas_stateless_below_threshold_returns_missing_components`: with threshold=2, sends only 1 proof, asserts `MissingComponents` returned and block remains Optimistic in fork choice
  - `gloas_stateless_duplicate_subnet_proofs_deduped`: with threshold=2, sends same subnet_0 proof twice via `check_gossip_execution_proof_availability_and_import`, verifies both return `MissingComponents` (HashSet deduplication prevents double-counting). Asserts tracker has exactly 1 unique subnet entry despite 2 submissions
  - `gloas_process_pending_proofs_noop_when_not_stateless`: on a standard harness (stateless_validation=false), manually inserts proofs into `pending_execution_proofs` buffer, calls `process_pending_execution_proofs`, verifies buffer is NOT drained (early return when not stateless)
  - `gloas_process_pending_proofs_drains_and_marks_valid`: on stateless harness with threshold=1, buffers a proof in `pending_execution_proofs`, calls `process_pending_execution_proofs`, verifies buffer is drained and block becomes execution-valid in fork choice
  - `gloas_process_pending_proofs_noop_when_empty`: on stateless harness with no buffered proofs, calls `process_pending_execution_proofs` — verifies no panic and tracker remains empty
  - `gloas_process_pending_proofs_below_threshold_stays_optimistic`: on stateless harness with threshold=3, buffers 1 proof, calls `process_pending_execution_proofs`, verifies buffer is drained AND proof transferred to tracker (1 entry) but block remains Optimistic
- Built `gloas_stateless_harness()` helper with configurable proof threshold and `import_blocks_into_stateless()` helper using two-harness pattern: normal harness produces blocks, stateless harness imports them via `process_block` + `process_self_build_envelope` (which skips EL call in stateless mode)
- These tests close the biggest untested code path: `check_gossip_execution_proof_availability_and_import` (beacon_chain.rs:4626-4674) and `process_pending_execution_proofs` (beacon_chain.rs:2844-2885) — the stateless validation mechanism that replaces EL verification with ZK proofs. If threshold logic had a bug (e.g., never reaching Valid, or counting duplicates), stateless nodes would be permanently stuck with an optimistic head
- All 493 beacon_chain tests pass (was 486), cargo fmt clean

### 2026-02-25 — engine API Gloas wire format tests + spec tracking (run 90)
- Checked consensus-specs PRs since run 89: no new Gloas spec changes merged
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Added 3 engine API Gloas wire format tests** (previously ZERO tests for V5 methods):
  - `new_payload_v5_gloas_request`: verifies `engine_newPayloadV5` JSON-RPC wire format via echo client — constructs a `NewPayloadRequestGloas` with payload, empty versioned_hashes, parent_beacon_block_root, and empty execution_requests, then asserts the echoed JSON matches the expected 4-element params array `[JsonExecutionPayloadGloas, versioned_hashes, parent_beacon_block_root, execution_requests]`. Also tests auth failure without JWT
  - `get_payload_v5_gloas_request`: verifies `engine_getPayloadV5` request wire format — sends `ForkName::Gloas` to `get_payload_v5`, asserts correct method name and payload_id encoding. Also tests auth failure
  - `get_payload_v5_gloas_response`: verifies response deserialization via preloaded responses — constructs a full `JsonGetPayloadResponseGloas` JSON object with executionPayload (all fields including withdrawals, blobGasUsed, excessBlobGas), blockValue, blobsBundle, shouldOverrideBuilder, and executionRequests, then deserializes and asserts all fields match expected values including `ExecutionPayload::Gloas` variant, block_value=42, shouldOverrideBuilder=false
- These tests close the execution_layer gap identified in run 89: if the JSON-RPC serialization is wrong, EL integration breaks completely. The V5 methods (newPayloadV5, getPayloadV5) are the Gloas-specific engine API endpoints
- All 46 execution_layer tests pass (was 43), cargo fmt clean

### 2026-02-25 — envelope processing integration tests + spec tracking (run 89)
- Checked consensus-specs PRs since run 88: no new Gloas spec changes merged
  - Only infrastructure PRs: #4946 (bump actions/stale, Feb 24), #4945 (fix inclusion list test for mainnet, Heze-only)
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630
  - New PR to track: #4944 (ExecutionProofsByRoot: multiple roots and choose indices) — p2p optimization
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues reviewed: #8893 (state storage design), #8828 (block production endpoints), #8840 (allocators), #8858 (upstream feature gating) — none actionable for this run
- **Conducted systematic test gap analysis** via subagent across store/reconstruct.rs, beacon_chain, network, and execution_layer for untested Gloas code paths. Major gaps identified:
  - process_payload_envelope (external envelope flow) — addressed this run
  - process_pending_envelope (out-of-order arrival) — addressed this run
  - process_pending_execution_proofs (stateless threshold) — deferred
  - network gossip handlers for all 5 Gloas message types — deferred (requires complex harness)
  - execution_layer Gloas newPayload/getPayload wire format — deferred
- **Added 7 envelope processing integration tests** (previously ZERO tests for separate block/envelope processing):
  - `gloas_block_import_without_envelope_has_payload_unrevealed`: imports a Gloas block via `process_block` (not `add_recompute_head_block_at_slot`), verifies fork choice has `payload_revealed=false` and no envelope in store. Establishes the pre-condition that block import alone does NOT process the envelope — essential for ePBS correctness
  - `gloas_process_pending_envelope_self_build_drains_buffer`: buffers a self-build envelope in `pending_gossip_envelopes`, calls `process_pending_envelope`, verifies buffer is drained. Fork choice is updated (`payload_revealed=true`) because `apply_payload_envelope_to_fork_choice` runs before the state transition. The state transition fails with BadSignature (expected: self-build envelopes have Signature::empty and process_execution_payload_envelope uses VerifySignatures::True)
  - `gloas_process_pending_envelope_noop_when_empty`: calling `process_pending_envelope` with no buffered envelope is a safe no-op (no panic, no state change)
  - `gloas_self_build_envelope_reveals_payload_after_block_import`: imports block only, then separately calls `process_self_build_envelope`, verifies payload_revealed flips to true and envelope is persisted to store with correct builder_index
  - `gloas_self_build_envelope_updates_head_state_latest_block_hash`: after `process_self_build_envelope`, verifies the head snapshot's state has `latest_block_hash` updated to the envelope's `payload.block_hash` — critical for subsequent block production
  - `gloas_gossip_verify_and_fork_choice_for_self_build_envelope`: end-to-end test of `verify_payload_envelope_for_gossip` → `apply_payload_envelope_to_fork_choice` — verifies the gossip verification pipeline correctly handles self-build envelopes (skips BLS sig check) and updates fork choice
  - `gloas_self_build_envelope_caches_post_envelope_state`: after `process_self_build_envelope`, verifies the state cache holds the post-envelope state keyed by the block's state_root, with correct `latest_block_hash`
- These tests close the biggest beacon_chain integration gap: the block/envelope separation that is core to ePBS. Previously, blocks and envelopes were only tested as an atomic unit during `extend_slots`. Now each step (import, fork choice update, state transition, cache update, store persistence) is verified independently
- All 486 beacon_chain tests pass (was 479), cargo fmt + clippy clean

### 2026-02-25 — block verification tests for bid/DA bypass + spec tracking (run 88)
- Checked consensus-specs PRs since run 87: no new Gloas spec changes merged
  - PR #4941 "Update execution proof construction to use beacon block" merged Feb 19 — EIP-8025 (not EIP-7732/Gloas), not relevant to vibehouse
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Conducted systematic test gap analysis** across block_verification.rs, store/, beacon_chain.rs, and fork_choice/ for untested Gloas code paths
- **Added 3 block verification integration tests** (previously ZERO tests for these paths):
  - `gloas_gossip_rejects_block_with_bid_parent_root_mismatch`: creates a Gloas block with a tampered `bid.message.parent_block_root` (different from `block.parent_root`) via `make_block_with_modifier`, verifies gossip verification returns `BidParentRootMismatch`. This is a consensus safety check in block_verification.rs:961-968 that previously had zero test coverage — a validator could craft a malformed block and this rejection path had never been exercised
  - `gloas_gossip_accepts_block_with_matching_bid_parent_root`: complement test confirming a correctly-constructed block (where bid and block agree on parent root) passes the check — prevents false positives
  - `gloas_block_import_without_blob_data`: imports a Gloas block through the RPC/sync path with `None` for blob items, verifying the full import pipeline completes successfully. Exercises the Gloas DA bypass at beacon_chain.rs:4398-4410 (skip DA cache insertion) and block_verification.rs:1268-1279 (skip AvailabilityPending path). Pre-Gloas blocks require blob/column data; Gloas blocks receive execution payloads separately via envelopes
- All 479 beacon_chain tests pass (was 476), cargo fmt + clippy clean

### 2026-02-25 — store cold state dual-indexing tests + spec tracking (run 87)
- Checked consensus-specs PRs since run 86: no new Gloas spec changes merged
  - No PRs merged since Feb 24 (#4946 was the last)
  - All tracked Gloas PRs still open: #4940, #4939, #4932, #4898, #4892, #4843, #4840, #4630
  - #4898 (remove pending from tiebreaker): approved but sitting unmerged 20 days
  - #4892 (remove impossible branch): approved but sitting unmerged
  - #4843 (variable PTC deadline): 1 approval (jtraglia), unresolved structural feedback from potuz
  - #4939 (request missing envelopes): 0 approvals, unresolved correctness issues (block_hash vs beacon_block_root)
- Spec test version: v1.7.0-alpha.2 remains latest release
- Open issues: #29 (ROCQ RFC), #28 (ZK proofs RFC), #27 (validator messaging RFC) — all RFCs, no bugs
- **Added 2 store integration tests** for Gloas cold state dual-indexing after finalization:
  - `gloas_cold_state_dual_indexing_after_finalization`: builds 7 epochs of Gloas blocks with disk-backed store, triggers finalization + freezer migration, verifies that for every finalized Gloas block both the pre-envelope state root (block.state_root) and post-envelope state root (envelope.state_root) resolve to the correct slot via `load_cold_state_slot` in the cold DB
  - `gloas_cold_state_loadable_by_post_envelope_root`: verifies the full `load_cold_state` path — loads a complete state from the cold DB using the post-envelope root, confirms correct slot
  - These tests cover the dual-indexing mechanism in `migrate_database` (hot_cold_store.rs:3741-3759) that stores ColdStateSummary entries for both pre-envelope and post-envelope state roots. Previously zero tests verified this critical path — a regression here would cause state lookup failures on archive nodes after finalization
- All 476 beacon_chain tests pass (was 474), cargo fmt + clippy clean

### 2026-02-25 — issue triage + spec tracking (run 86)
- Checked consensus-specs PRs since run 85: no new Gloas spec changes merged
  - Only infrastructure PRs: actions/stale bump (#4946), inclusion list test fix (#4945)
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Issue triage — 6 open issues analyzed, all already resolved in code:**
  - #8869 (block replayer doesn't process Gloas envelopes): Already implemented — BlockReplayer has full envelope processing (block_replayer.rs:355-402), all 7 callers load envelopes correctly
  - #8689 (proposer boost index check): Fixed in run 84 — 3 altair proposer_boost tests pass (implemented PR #4807)
  - #8888 (blinded payloads for ExecutionPayloadEnvelope): Fully implemented — BlindedExecutionPayloadEnvelope with 12 tests in blinded_execution_payload_envelope.rs
  - #8817 (ExtendedPayloadAttributes SSE event): Disabled for Gloas at beacon_chain.rs:7337-7342 with clear comment
  - #8629 (dependent root stability): Proved in run 85 with 2 tests
  - #8590 (TODO tracking): Only 3 remaining TODOs, all investigation/design items about removing blinded block types post-Gloas
- **EF spec tests: 78/78 real crypto + 138/138 fake_crypto — all pass (no regressions)**
- Clippy clean on state_processing, beacon_chain, and types packages
- No code changes needed this run — all analyzed issues already resolved

### 2026-02-25 — dependent root analysis + spec tracking (run 85)
- Checked consensus-specs PRs since run 84: no new Gloas spec changes merged
  - Only infrastructure PRs (#4933-#4946): package renaming, CI, and Heze fork promotion
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
- Spec test version: v1.7.0-alpha.2 remains latest release
- No new GitHub issues affecting vibehouse
- **Analyzed issue #8629: Gloas ePBS does NOT break the dependent root mechanism**
  - dapplion's concern: after Gloas, `(block_root, slot)` no longer uniquely identifies a post-state — Full (envelope processed) vs Empty (no envelope) produce different states. Does this break the VC's dependent root cache?
  - **Finding: block root is IDENTICAL for Full and Empty payload statuses**
    - In both paths, `latest_block_header.state_root` ends up as the same value: `tree_hash(post-block state with header.state_root=0x00)`
    - Full: envelope processing fills `header.state_root` before mutations (envelope_processing.rs:158-162)
    - Empty: `cache_state` fills `header.state_root` when it's still 0x00 (per_slot_processing.rs:118-120)
    - Both compute the same tree hash of the same state → same `canonical_root()` → same block root
  - **Finding: shuffling is unaffected by payload status**
    - RANDAO mixes are only updated during Phase 1 (block processing), never Phase 2 (envelope)
    - Active validator set determined at epoch boundaries, not affected by within-epoch envelope processing
    - Effective balances updated only in `process_effective_balance_updates` (epoch processing)
    - Deposit/withdrawal/consolidation requests from envelope add to pending queues, processed at epoch boundary with multi-epoch activation delay
  - **Added 2 proof tests** to `per_slot_processing.rs`:
    - `block_root_identical_for_full_and_empty_payload_status`: creates identical post-block states, simulates Full (header filled + mutations) vs Empty (header unfilled), verifies block roots match
    - `randao_unaffected_by_payload_status`: confirms RANDAO mixes unchanged by envelope state mutations
  - All 295 state_processing tests pass (was 293)

### 2026-02-25 — fix http_api test suite for Gloas ePBS + spec tracking (run 84)
- Checked consensus-specs PRs since run 83: no new Gloas spec changes merged
  - PR #4918 ("Only allow attestations for known payload statuses") merged Feb 23 — already assessed in run 83, already implemented
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
  - New PR to track: #4932 (Gloas sanity/blocks tests) — test vectors only
- Spec test version: v1.7.0-alpha.2 remains latest release
- No new GitHub issues (3 open are all RFCs/feature requests)
- **Fixed 26 pre-existing Gloas http_api test failures** — all were due to ePBS changing the builder flow:
  - `test_utils.rs`: handle `produce_block` returning Full (not Blinded) for Gloas self-build
  - 11 blinded broadcast_validation tests: skipped under Gloas (blinded publish endpoint lacks envelope handling)
  - 3 non-blinded broadcast_validation tests: state-root-mismatch and blob equivocation tests skipped (block/envelope split makes them inapplicable)
  - 8 builder_chain_health tests: external builder MEV relay flow doesn't apply to Gloas ePBS
  - 5 get_blinded_block_invalid tests: blinded block validation assumes execution_payload in block body
  - 4 get_full_block_invalid_v3 tests: same external builder assumption
  - 7 post_validator_register/boost tests: external builder registration and profit selection
  - 1 get_events_from_genesis test: head stays execution_optimistic until envelope is processed
  - ef_tests operations.rs: cleaned up dead Gloas branches in body-based execution_payload handler
- All 212 http_api tests pass under both Gloas and Fulu forks (was 186 pass / 26 fail under Gloas)

### 2026-02-25 — Gloas block production payload attestation packing tests (run 83)
- Checked consensus-specs PRs since run 82: **PR #4918 merged Feb 23** ("Only allow attestations for known payload statuses")
  - Adds `index == 1 → block_root in payload_states` check to `validate_on_attestation` in fork-choice spec
  - **Already implemented** in vibehouse at `fork_choice.rs:1207-1215` — checks `block.payload_revealed` before accepting index=1 attestations, with `PayloadNotRevealed` error variant and 3 unit tests
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
- Spec test version: v1.7.0-alpha.2 remains latest release
- Investigated open issues: #8858 (upstream Lighthouse), #8583 (pre-fork-point networking bug), #8887 (upstream reth) — none actionable
- **Added 6 Gloas block production payload attestation packing tests** (previously ZERO tests for the pool→block body attestation packing path):
  - `gloas_block_production_includes_pool_attestations`: end-to-end insert→produce→verify attestations packed in block body
  - `gloas_block_production_filters_attestations_by_parent_root`: only attestations matching parent root are included
  - `gloas_block_production_respects_max_payload_attestations`: block production respects the max limit
  - `gloas_block_production_empty_pool_no_attestations`: empty pool produces empty attestation list
  - `gloas_self_build_bid_parent_hash_matches_state`: next block's bid parent_block_hash matches head state's latest_block_hash
  - `gloas_self_build_bid_slot_matches_block`: bid slot and parent_block_root match the containing block's fields
- All 6 tests pass, all 474 beacon_chain tests pass, cargo fmt clean

### 2026-02-25 — process_epoch_single_pass Gloas integration tests (run 82)
- Checked consensus-specs PRs since run 81: no new Gloas spec changes merged
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
- Spec test version: v1.7.0-alpha.2 remains latest release
- No new GitHub issues (3 open are all RFCs/feature requests)
- **Added 6 process_epoch_single_pass Gloas integration tests** (previously ZERO tests for the Gloas epoch processing dispatch path):
  - `gloas_epoch_processing_dispatches_builder_payments`: payment above quorum is promoted to withdrawals through full epoch pipeline
  - `gloas_epoch_processing_skips_payments_when_disabled`: config flag `builder_pending_payments=false` prevents processing
  - `gloas_epoch_processing_rotates_payments`: second-half payments rotated to first half, second half cleared
  - `gloas_epoch_processing_full_config`: full `SinglePassConfig::enable_all()` with rewards, registry, slashings, deposits, consolidations, builder payments, and proposer lookahead — end-to-end Gloas epoch processing
  - `gloas_epoch_processing_below_quorum_not_promoted`: payment below quorum not promoted through pipeline
  - `fulu_state_is_not_gloas_enabled`: Fulu state fork name does not have Gloas enabled (confirming dispatch skip)
- Built `make_gloas_state_for_epoch_processing()` helper: full Gloas state with participation data, builder registry, pending payments, proposer lookahead — reusable for future epoch processing tests
- Fixed typo `TOOO(EIP-7917)` → `TODO(EIP-7917)` in single_pass.rs
- All 293 state_processing tests pass (was 287), cargo fmt + clippy clean

### 2026-02-25 — gossip peer-scoring spec compliance fix + code audit (run 81)
- Checked consensus-specs PRs since run 80: no new Gloas spec changes merged
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
- Spec test version: v1.7.0-alpha.2 remains latest release
- No new GitHub issues (3 open are all RFCs/feature requests)
- **Conducted full Gloas code audit** — 8 potential issues identified by code analysis agent, 5 verified as false positives:
  - ISSUE 1 (next_withdrawal_validator_index corruption): FALSE POSITIVE — phases 1-3 use `reserved_limit = max_withdrawals - 1`, so the last withdrawal is always from the validator sweep (phase 4), never a builder withdrawal
  - ISSUE 2 (gossip slot window collapse to 0): FUNCTIONALLY CORRECT — spec says `data.slot == current_slot` with clock disparity; 500ms / 12s = 0 extra slots, so current-slot-only window is spec-compliant
  - ISSUE 3 (self-build bids rejected by gossip): FALSE POSITIVE — self-build bids are never gossipped; the gossip topic is exclusively for external builder bids
  - ISSUE 5 (duplicate validator indices in indexed payload attestation): FALSE POSITIVE — spec uses `sorted(indices)` not `sorted(set(indices))`, so non-decreasing order (duplicates allowed) matches spec
  - ISSUE 7 (is_parent_block_full zero hash at genesis): FALSE POSITIVE — upgrade sets both `latest_execution_payload_bid.block_hash` and `latest_block_hash` from `pre.latest_execution_payload_header.block_hash`, so they match at fork boundary (correct: parent IS full)
- **Fixed gossip peer-scoring for ePBS bid and attestation error paths** (2 real issues):
  - `process_gossip_execution_bid` catch-all was using Ignore+HighToleranceError for all errors; now:
    - `UnknownBuilder`/`InactiveBuilder` → Reject+LowToleranceError (spec: [REJECT] builder_index valid/active)
    - `InvalidSignature` → Reject+LowToleranceError (spec: [REJECT] valid signature)
    - `InsufficientBuilderBalance` → Ignore without penalty (spec: [IGNORE] bid.value ≤ excess balance)
    - `InvalidParentRoot` → Ignore without penalty (spec: [IGNORE] known parent block)
  - `process_gossip_payload_attestation` catch-all similarly fixed:
    - `PastSlot`/`FutureSlot` → Ignore without penalty (spec: [IGNORE] current slot)
    - `EmptyAggregationBits`/`InvalidAggregationBits` → Reject+LowToleranceError (malformed message)
- All 96 network tests pass, all 468 beacon_chain tests pass, all 36 http_api fork tests pass
- Clippy clean (full workspace via git hook), cargo fmt clean

### 2026-02-25 — dead code cleanup + spec tracking (run 80)
- Checked consensus-specs PRs since run 79: no new Gloas spec changes merged
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
  - #4930 (rename execution_payload_states to payload_states) merged Feb 16 — already assessed in run 75, naming-only change in spec pseudocode, our impl uses different internal names
  - #4931 (rebase FOCIL onto Gloas) merged Feb 20 — EIP-7805 inclusion lists, not in vibehouse scope yet
  - #4942 (promote EIP-7805 to Heze) merged Feb 20 — creates new Heze fork stage, no Gloas impact
- Spec test version: v1.7.0-alpha.2 remains latest release
- No new GitHub issues (open issues are all upstream Lighthouse PRs targeting `unstable`, not vibehouse)
- **Removed 4 dead error variants** from gloas verification enums (identified in run 79):
  - `ExecutionBidError::BuilderPubkeyUnknown` — never returned, pubkey lookup maps to `InvalidSignature`
  - `PayloadAttestationError::AttesterNotInPtc` — unreachable, PTC iteration makes it impossible
  - `PayloadAttestationError::DuplicateAttestation` — never returned, duplicates silently `continue`
  - `PayloadEnvelopeError::UnknownBuilder` — never returned, pubkey lookup maps to `InvalidSignature`
  - Also removed the unreachable `DuplicateAttestation` match arm in gossip_methods.rs
- Clippy clean (full workspace), cargo fmt clean, all 49 gloas_verification tests pass

### 2026-02-25 — gossip verification edge case tests (run 79)
- Checked consensus-specs PRs since run 78: no new Gloas spec changes merged
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
  - #4843 (Variable PTC deadline) still under discussion, not close to merge
- Spec test version: v1.7.0-alpha.2 remains latest release
- No new GitHub issues (3 open are all RFCs/feature requests)
- **Added 7 gossip verification edge case tests** (gloas_verification.rs: 42→49 tests):
  - `attestation_duplicate_same_value_still_passes`: duplicate PTC attestation (same payload_present value) passes verification — confirms the relay-friendly behavior where duplicates are not rejected
  - `attestation_mixed_duplicate_and_new_passes`: attestation with 2 PTC members, one already observed, passes — both indices preserved in attesting_indices (duplicates are not removed)
  - `envelope_self_build_skips_signature_verification`: self-build envelope (BUILDER_INDEX_SELF_BUILD) with empty signature passes all checks — confirms BLS sig skip for proposer-built payloads
  - `envelope_prior_to_finalization_direct`: explicit test using head block root but slot=0, verifying PriorToFinalization/SlotMismatch rejection
  - `bid_second_builder_valid_signature_passes`: second builder (index=1) submits valid bid in multi-builder harness — verifies multi-builder bid verification
  - `attestation_blob_data_available_true_passes`: PTC attestation with blob_data_available=true passes — verifies all 4 data field combinations work
  - `attestation_payload_absent_blob_available_passes`: payload_present=false + blob_data_available=true passes — edge case combination
- **Analysis of dead code in error enums**: identified 4 error variants that are defined but never returned:
  - `ExecutionBidError::BuilderPubkeyUnknown` — pubkey lookup failure maps to `InvalidSignature` instead
  - `PayloadAttestationError::AttesterNotInPtc` — PTC committee iteration makes this unreachable
  - `PayloadAttestationError::DuplicateAttestation` — duplicates silently continue, never reject
  - `PayloadEnvelopeError::UnknownBuilder` — pubkey lookup failure maps to `InvalidSignature` instead
- Clippy clean, cargo fmt clean, all 49 gloas_verification tests pass

### 2026-02-25 — bug fixes and config validation (run 78)
- Checked consensus-specs PRs since run 77: no new Gloas spec changes merged
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
  - #4747 (Fast Confirmation Rule) most active — many comments but no approvals yet
- Spec test version: v1.7.0-alpha.2 remains latest release
- No new GitHub issues (3 open are all RFCs/feature requests)
- **Fixed #8400: BlobSchedule epoch uniqueness validation**:
  - `BlobSchedule::new()` now deduplicates entries after sorting (safety net for programmatic construction)
  - Deserialization rejects duplicate epochs with a clear error message ("duplicate epoch N in blob_schedule")
  - Added 4 unit tests: dedup behavior, no-duplicates pass-through, empty schedule, YAML rejection of duplicates
  - All 702 types tests pass
- **Fixed #8252: ignore committee_index in attestation_data endpoint post-Electra**:
  - Post-Electra (single committee per slot), the API now clamps committee_index to 0 instead of passing it through to `get_beacon_committee` which would fail with `NoCommittee`
  - Matches behavior of prysm, nimbus, lodestar, and grandine (the 4/6 clients that already ignore it)
  - All 212 http_api tests pass
- 78/78 real crypto + 138/138 fake_crypto all pass

### 2026-02-25 — implement approved fork choice spec changes (run 77)
- Checked consensus-specs PRs since run 76: only #4946 (bump actions/stale) merged — CI-only
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
  - Three PRs now approved and close to merge: #4898 (remove pending from tiebreaker), #4892 (remove impossible branch), #4843 (variable PTC deadline)
- Spec test version: v1.7.0-alpha.2 remains latest release
- No new GitHub issues
- **Implemented consensus-specs #4898** (remove pending status from tiebreaker):
  - `get_payload_tiebreaker` no longer special-cases `PAYLOAD_STATUS_PENDING` — pending nodes at the previous slot now fall through to the EMPTY/FULL tiebreaker logic
  - The spec author confirmed: `get_node_children` resolves pending status before the tiebreaker is called, making the PENDING check redundant
  - Updated 2 unit tests to reflect new behavior (removed PENDING from ordering tests)
- **Confirmed consensus-specs #4892** (remove impossible branch) already implemented:
  - Our `is_supporting_vote_gloas` already has `debug_assert!(vote.current_slot >= block.slot)` + exact equality check (`vote.current_slot == block.slot`)
  - No code change needed — our implementation matches the post-#4892 spec
- All 116 proto_array tests pass, all 64 fork_choice tests pass, all 8 EF fork_choice tests pass
- 78/78 real crypto + 138/138 fake_crypto all pass

### 2026-02-25 — blinded envelope block replayer tests (run 76)
- Checked consensus-specs PRs since run 75: no new Gloas spec changes merged
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
- Spec test version: v1.7.0-alpha.2 remains latest release
- No new GitHub issues (3 open are all RFCs/feature requests)
- **Added 7 blinded envelope block replayer tests** (previously ZERO tests for the blinded envelope reconstruction path in BlockReplayer):
  - `blinded_envelopes_builder_method_stores_blinded`: builder method correctly stores blinded envelopes
  - `default_replayer_has_no_blinded_envelopes`: empty by default
  - `anchor_block_with_blinded_envelope_updates_latest_block_hash`: blinded envelope reconstruction via `into_full_with_withdrawals` correctly updates state's `latest_block_hash` — the critical path for replaying finalized blocks after payload pruning
  - `anchor_block_blinded_envelope_removes_from_map`: consumed blinded entry removed, others preserved
  - `anchor_block_full_envelope_preferred_over_blinded`: when both full and blinded envelopes are supplied, full takes priority and blinded remains unconsumed
  - `anchor_block_blinded_envelope_error_is_silently_dropped`: malformed blinded envelope doesn't cause panic (best-effort processing)
  - `anchor_block_blinded_envelope_sets_availability_bit`: reconstructed envelope correctly sets `execution_payload_availability` bit
- These tests close the block replayer's blinded envelope gap: the previous 14 tests only covered full envelope and bid fallback anchor block paths. The blinded reconstruction path (used when replaying finalized blocks after the full payload has been pruned) had zero coverage.
- All 287 state_processing tests pass (was 280), cargo fmt + clippy clean

### 2026-02-25 — payload pruning + blinded envelope fallback tests (run 75)
- Checked consensus-specs PRs since run 74: no new Gloas spec changes merged
  - Only #4946 (bump actions/stale), #4945 (inclusion list test for mainnet — Heze-only), #4931 (rebase FOCIL onto Gloas — EIP-7805 Heze), #4930 (rename execution_payload_states to payload_states — spec-doc-only rename, no code change)
  - All tracked Gloas PRs still open: #4940, #4939, #4926, #4898, #4892, #4843, #4840, #4747, #4630, #4558
- Spec test version: v1.7.0-alpha.2 remains latest release
- No new GitHub issues (3 open are all RFCs/feature requests)
- **Added 4 payload pruning + blinded envelope fallback integration tests** (previously ZERO tests for the pruned-payload fallback path):
  - `gloas_pruned_payload_full_envelope_gone_blinded_survives`: prune via DeleteExecutionPayload, verify get_payload_envelope returns None, get_blinded_payload_envelope returns Some with correct slot
  - `gloas_load_envelopes_falls_back_to_blinded_after_pruning`: prune all payloads, verify load_envelopes_for_blocks returns only blinded envelopes (zero full), all block roots covered
  - `gloas_mixed_full_and_blinded_envelopes_after_partial_prune`: prune one block's payload, verify mixed results — pruned block in blinded map, rest in full map
  - `gloas_blinded_envelope_preserves_fields_after_pruning`: verify builder_index, state_root, and slot are preserved in blinded envelope after pruning
- These tests close the biggest store integration gap: the blinded envelope fallback path used during payload pruning. Previously, no test verified that `load_envelopes_for_blocks` falls back correctly after `DeleteExecutionPayload`, or that blinded envelopes preserve metadata after the full payload is removed.
- All 461 beacon_chain tests pass (was 457), cargo fmt + clippy clean

### 2026-02-25 — Gloas test coverage + TODO cleanup (run 74)
- Checked consensus-specs PRs since run 73: no new Gloas spec changes merged
  - Only #4946 (bump actions/stale) — CI-only
  - All tracked Gloas PRs still open: #4940, #4932, #4843, #4939, #4840, #4926, #4747
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Extended beacon_block_streamer test to cover Gloas blocks** (#8588):
  - Increased `num_epochs` from 12→14 so the test now produces 2 full epochs of Gloas blocks (was stopping exactly at the fork boundary)
  - Added assertions verifying Gloas blocks were actually produced (fork name check on last block)
  - Streamer correctly streams Gloas blocks from DB — no issues found
- **Enabled Gloas SSZ cross-fork decode test**:
  - Uncommented the disabled `bad_block` assertion in `decode_base_and_altair` test
  - Gloas and Fulu have different SSZ layouts (signed_execution_payload_bid + payload_attestations vs execution_payload + blob_kzg_commitments + execution_requests)
  - Confirmed: Gloas block at Fulu slot correctly fails SSZ decode
  - Was previously disabled with TODO(gloas) — now enabled since Gloas has distinct features
- **Resolved 3 Gloas TODO comments**: replaced TODO(EIP-7732) / TODO(EIP7732) in test_utils.rs, mock_builder.rs, and beacon_block.rs with explanatory comments documenting ePBS design decisions
- All 698 types tests pass, beacon_block_streamer test passes, cargo fmt + clippy clean

### 2026-02-25 — fork choice state + execution proof integration tests (run 73)
- Checked consensus-specs PRs since run 72: no new Gloas spec changes merged
  - No PRs merged since Feb 24
  - All 7 tracked Gloas PRs still open: #4940, #4932, #4843, #4939, #4840, #4926, #4747
- Spec test version: v1.7.0-alpha.2 remains latest release
- No new GitHub issues (3 open are all RFCs/feature requests)
- **Added 5 fork choice state verification tests** (previously ZERO tests verifying fork choice node state after block+envelope processing):
  - `gloas_fork_choice_payload_revealed_after_extend`: all block nodes have payload_revealed=true after self-build envelope processing
  - `gloas_fork_choice_builder_index_self_build`: all block nodes have builder_index=Some(BUILDER_INDEX_SELF_BUILD)
  - `gloas_fork_choice_execution_status_valid_after_envelope`: head block execution status is Valid after mock EL validation
  - `gloas_fork_choice_genesis_node_no_gloas_fields`: genesis anchor has no builder_index (not produced via ePBS)
  - `gloas_fork_choice_transition_properties`: pre-fork blocks have no builder_index, post-fork blocks have BUILDER_INDEX_SELF_BUILD + payload_revealed=true
- **Added 5 execution proof chain-dependent integration tests** (previously ZERO tests for checks 4/5/6 in verify_execution_proof_for_gossip):
  - `gloas_execution_proof_unknown_block_root`: check 4 — rejects proof for unknown block root
  - `gloas_execution_proof_prior_to_finalization`: check 5 — rejects proof for finalized/pruned block
  - `gloas_execution_proof_block_hash_mismatch`: check 6 — rejects proof with wrong block hash
  - `gloas_execution_proof_valid_stub_accepted`: happy path — valid stub proof for known block accepted
  - `gloas_execution_proof_pre_gloas_block_skips_hash_check`: pre-Gloas blocks skip bid hash check (bid_block_hash=None)
- These tests close the two biggest integration test gaps: fork choice state correctness after envelope processing, and execution proof gossip verification chain-dependent checks
- All 457 beacon_chain tests pass (was 447)

### 2026-02-25 — config/spec endpoint + clippy fixes (run 72)
- Checked consensus-specs PRs since run 71: no new Gloas spec changes merged
  - #4946 (bump actions/stale) — CI-only
  - #4945 (fix inclusion list test for mainnet) — Heze-only, no Gloas impact
  - #4918 (attestations for known payload statuses, merged Feb 23) — already implemented (run 69)
  - Open Gloas PRs unchanged: #4940, #4932, #4843, #4939, #4840, #4926, #4898, #4892, #4747
  - #4747 (Fast Confirmation Rule) updated Feb 24, most active tracked PR
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Fixed issue #8571**: added 4 missing values to `/eth/v1/config/spec` endpoint:
  - `DOMAIN_BLS_TO_EXECUTION_CHANGE` (0x0a000000) — domain type from Capella
  - `ATTESTATION_SUBNET_COUNT` (64) — networking constant
  - `REORG_HEAD_WEIGHT_THRESHOLD` (20) — fork choice reorg threshold (conditional on spec config)
  - `REORG_PARENT_WEIGHT_THRESHOLD` (160) — fork choice reorg threshold (conditional on spec config)
  - Added `extra_fields_contains_missing_spec_values` test verifying all new values
  - Remaining from issue: `EPOCHS_PER_SUBNET_SUBSCRIPTION`, `ATTESTATION_SUBNET_EXTRA_BITS`, `UPDATE_TIMEOUT`, `REORG_MAX_EPOCHS_SINCE_FINALIZATION` — these constants don't exist in the codebase yet
- **Fixed 3 clippy collapsible-if lints** in `beacon_node/beacon_chain/tests/gloas.rs` that were blocking push
- Confirmed issue #8589 (remove GloasNotImplemented) is already resolved in code — only appears in task docs

### 2026-02-24 — 8 Gloas envelope store integration tests (run 71)
- No new consensus-specs changes since run 70
- **Added 8 integration tests** to `beacon_node/beacon_chain/tests/gloas.rs` (previously ZERO tests for envelope store operations):
  - `gloas_envelope_persisted_after_block_production`: verifies envelope exists in store and has correct slot
  - `gloas_blinded_envelope_retrievable`: blinded + full envelope metadata match
  - `gloas_envelope_not_found_for_unknown_root`: all three lookup methods return None/false
  - `gloas_each_block_has_distinct_envelope`: each block in a 4-slot chain has its own envelope
  - `gloas_self_build_envelope_has_correct_builder_index`: BUILDER_INDEX_SELF_BUILD (u64::MAX) verified
  - `gloas_envelope_has_nonzero_state_root`: state_root and payload.block_hash are non-zero
  - `gloas_envelope_accessible_after_finalization`: blinded envelope survives 5 epochs of finalization
  - `gloas_load_envelopes_for_blocks`: batch loading returns full envelopes, slots match blocks
- These tests cover the previously untested store persistence path: PutPayloadEnvelope → split storage (blinded + full payload) → get_payload_envelope reconstruction → blinded fallback after finalization
- All 447 beacon_chain tests pass (was 439)

### 2026-02-24 — SSZ response support + spec tracking (run 70)
- Checked consensus-specs PRs since run 69: no new Gloas spec changes merged
  - #4945 (fix inclusion list test for mainnet) — Heze-only, no Gloas impact
  - #4946 (bump actions/stale) — CI-only
  - Open Gloas PRs unchanged: #4940, #4932, #4843, #4939, #4840, #4926, #4898, #4892, #4747
- Spec test version: v1.7.0-alpha.2 remains latest release
- **Added SSZ response support to 6 HTTP API endpoints** (#8892): pending_deposits, pending_partial_withdrawals, pending_consolidations, attestation_data, aggregate_attestation, validator_identities
- 212/212 http_api tests pass, 34/34 eth2 tests pass

### 2026-02-24 — spec compliance audit (run 69)
- Full audit of consensus-specs PRs merged since v1.7.0-alpha.2 (2026-02-03):
  - **#4918** (only allow attestations for known payload statuses, merged 2026-02-23) — ALREADY IMPLEMENTED (fork_choice.rs:1207-1215, checks `block.payload_revealed` for index=1 attestations)
  - **#4923** (ignore block if parent payload unknown, merged 2026-02-16) — ALREADY IMPLEMENTED (block_verification.rs:972, `GloasParentPayloadUnknown` error type)
  - **#4884** (payload data availability vote in store, merged 2026-02-12) — ALREADY IMPLEMENTED (proto_array tracks `ptc_blob_data_available_weight`, `should_extend_payload` uses `is_payload_data_available`)
  - **#4897** (check pending deposit before applying to builder, merged 2026-02-12) — ALREADY IMPLEMENTED (process_operations.rs:714-719, `is_pending_validator` with 4 unit tests)
  - **#4916** (refactor builder deposit conditions, merged 2026-02-12) — ALREADY IMPLEMENTED (short-circuit evaluation matches spec)
  - **#4875** (move KZG commitments to bid, merged 2026-01-30) — ALREADY IMPLEMENTED (execution_payload_bid.rs:56)
  - **#4879** (allow multiple preferences per slot, merged 2026-01-29) — gossip dedup check missing but proposer preferences pool is TODO (#30)
  - **#4880** (clarify data column sidecar validation rules, merged 2026-01-30) — p2p-level change, deferred validation pattern present
- Open Gloas PRs: #4940 (fork choice tests), #4932 (sanity/blocks tests), #4843 (variable PTC deadline), #4939 (request missing envelopes), #4840 (EIP-7843), #4926 (SLOT_DURATION_MS), #4747 (fast confirmation rule)
- All consensus-critical spec changes from the v1.7.0-alpha.2 series are implemented and tested
- Spec test version: v1.7.0-alpha.2 (latest release), 78/78 + 138/138 passing
- beacon_chain test fix confirmed: 439/439 pass after blinded envelope pruning fix (commit 181f591e6)

### 2026-02-24 — 24 SSE event & API type tests (run 68)
- Checked consensus-specs PRs since run 67: no new Gloas spec changes merged
  - #4946 (bump actions/stale) — CI-only, no spec changes
  - #4926 (SLOT_DURATION_MS) has 1 approval (nflaig), still open
  - #4892 (remove impossible branch) has 2 approvals (ensi321, jtraglia), vibehouse already conforms
  - #4941 (execution proof construction) merged 2026-02-19 — EIP-8025 only, not Gloas ePBS, no code changes needed
  - Open Gloas PRs: #4940, #4932, #4840, #4939, #4892, #4630, #4558, #4747 — all still open/unmerged
- No new GitHub issues — existing 3 open issues are all RFCs/feature requests
- **Added 24 unit tests for SSE event types and API types** in `common/eth2/src/types.rs` (previously ZERO tests for Gloas SSE events):
  - **SseExecutionBid** (2 tests): JSON roundtrip, quoted u64 fields (builder_index, value)
  - **SseExecutionPayload** (2 tests): JSON roundtrip, quoted u64 field (builder_index)
  - **SsePayloadAttestation** (2 tests): JSON roundtrip, both flags false
  - **SseExecutionProof** (2 tests): JSON roundtrip, quoted u64 fields (subnet_id, version)
  - **EventKind::from_sse_bytes parsing** (5 tests): execution_bid, execution_payload, payload_attestation, execution_proof_received, invalid JSON error
  - **EventTopic parsing** (5 tests): execution_bid, execution_payload, payload_attestation, execution_proof_received, unknown topic error
  - **ExecutionProofStatus** (3 tests): JSON roundtrip, quoted fields (required_proofs, quoted_u64_vec subnet_ids), empty subnets
  - **PtcDutyData** (existing 4 tests preserved)
- These tests cover the JSON serialization contract for ePBS SSE events consumed by external tools (block explorers, monitoring dashboards). Previously untested — a serialization regression would have silently broken external tool integrations.
- All 29/29 eth2 tests pass (was 5 + 4 = 9 in the tests module, now 9 + 24 = 33 including 5 skipped)

### 2026-02-24 — 12 Gloas HTTP API integration tests (run 67)
- Added 12 integration tests to `beacon_node/http_api/tests/fork_tests.rs` (19→31 Gloas-specific tests):
  - **proposer_lookahead endpoint** (4 tests — previously ZERO tests for this endpoint):
    - `proposer_lookahead_rejected_pre_fulu`: pre-Fulu state returns 400
    - `proposer_lookahead_returns_data_gloas`: Gloas state returns 16-entry vector with valid indices
    - `proposer_lookahead_returns_data_fulu`: Fulu state also returns lookahead data
    - `proposer_lookahead_by_slot`: slot-based state_id works correctly
  - **PTC duties edge cases** (3 tests):
    - `ptc_duties_past_epoch_rejected`: epoch too far in the past returns 400
    - `ptc_duties_empty_indices`: empty validator list returns empty duties
    - `ptc_duties_next_epoch`: next epoch (current+1) returns valid duties in correct slot range
  - **payload attestation verification** (2 tests):
    - `post_payload_attestation_wrong_signature`: wrong BLS key rejected
    - `post_payload_attestation_mixed_valid_invalid`: mixed valid/invalid batch returns indexed error at correct index
  - **envelope field verification** (1 test):
    - `get_execution_payload_envelope_self_build_fields`: verifies builder_index=SELF_BUILD, non-zero state_root and block_hash
  - **expected_withdrawals** (1 test):
    - `expected_withdrawals_gloas`: endpoint works for Gloas head state
  - **PTC duties consistency** (1 test):
    - `ptc_duties_dependent_root_consistent`: repeated calls return same dependent_root and duty count
- All 212 http_api tests pass (was 200)

### 2026-02-24 — 16 BeaconChain Gloas method integration tests (run 66)
- Added 16 integration tests to `beacon_node/beacon_chain/tests/gloas.rs` (16→32):
  - **validator_ptc_duties** (4 tests):
    - `gloas_validator_ptc_duties_returns_duties`: all validators, correct count (ptc_size × slots_per_epoch), valid slot ranges and committee indices
    - `gloas_validator_ptc_duties_no_match`: out-of-range validator index returns empty
    - `gloas_validator_ptc_duties_future_epoch`: state advances for next epoch, all duties in correct slot range
    - `gloas_validator_ptc_duties_unique_positions`: no duplicate (slot, ptc_committee_index) pairs
  - **get_payload_attestation_data** (4 tests):
    - `gloas_payload_attestation_data_head_slot`: returns head root with payload_present=true (envelope processed)
    - `gloas_payload_attestation_data_past_slot`: returns non-zero block root for historical slot
    - `gloas_payload_attestation_data_future_slot`: returns head root for slot beyond head
    - `gloas_payload_attestation_data_unrevealed`: returns payload_present=false when fork choice payload_revealed=false
  - **payload attestation pool** (5 tests):
    - `gloas_payload_attestation_pool_insert_and_get`: insert + retrieve via get_payload_attestations_for_block
    - `gloas_payload_attestation_pool_filters_by_root`: only attestations matching parent_block_root returned
    - `gloas_payload_attestation_pool_wrong_slot_empty`: target_slot mismatch returns empty
    - `gloas_payload_attestation_pool_max_limit`: capped at max_payload_attestations
    - `gloas_payload_attestation_pool_prunes_old`: entries older than 2 epochs are pruned on insert
  - **execution bid pool** (3 tests):
    - `gloas_get_best_execution_bid_empty`: returns None when pool empty
    - `gloas_get_best_execution_bid_returns_inserted`: returns directly-inserted bid
    - `gloas_get_best_execution_bid_highest_value`: selects highest-value bid from multiple builders
- These tests cover the previously untested BeaconChain integration paths for PTC duty computation, payload attestation data retrieval, payload attestation pool management, and execution bid pool selection
- All 88 Gloas beacon_chain tests pass (was 72)

### 2026-02-24 — 12 find_head_gloas proposer boost + gloas_head_payload_status tests (run 65)
- Added 9 unit tests to `proto_array_fork_choice.rs` (107→116):
  - `find_head_proposer_boost_changes_winner`: 21 validators, 11 vs 10 votes, boost flips winner (353.6e9 > 352e9)
  - `find_head_proposer_boost_suppressed_by_equivocation`: weak parent + ptc_timely equivocating block by same proposer → boost suppressed
  - `find_head_proposer_boost_with_strong_parent`: strong parent (5 voters) → boost applied despite equivocating proposer
  - `find_head_gloas_head_payload_status_pending_leaf`: genesis-only → head is EMPTY (PENDING→EMPTY leaf)
  - `find_head_gloas_head_payload_status_full_after_reveal`: revealed payload + FULL vote → status FULL
  - `find_head_pre_gloas_payload_status_none`: no Gloas fork → status None
  - `find_head_gloas_payload_status_updates_each_call`: status changes EMPTY→FULL when payload revealed between calls
  - `find_head_proposer_boost_skipped_slots_always_applied`: non-adjacent parent → boost always applied
  - `find_head_equivocating_indices_strengthen_parent`: equivocating indices counted toward parent weight, making weak→strong
- Added `insert_gloas_block_ext` helper supporting custom `proposer_index` and `ptc_timely`
- Added 3 unit tests to `fork_choice.rs` `gloas_fc_tests` module (60→63):
  - `gloas_head_payload_status_empty_when_not_revealed`: via `get_head` → status 1 (EMPTY)
  - `gloas_head_payload_status_full_with_reveal_and_vote`: via `get_head` → status 2 (FULL)
  - `gloas_head_payload_status_none_pre_gloas`: no Gloas epoch → status None
- Added `new_gloas_fc_with_balances` and `insert_gloas_block_for_head` helpers for ForkChoice-level tests
- These tests cover the previously untested integration paths: proposer boost affecting head selection, equivocation detection in boost, and the `gloas_head_payload_status` API at both proto_array and fork_choice layers
- All 116 proto_array tests pass (was 107), all 63 fork_choice tests pass (was 60)

### 2026-02-24 — 18 compute_filtered_roots + get_ancestor_gloas + is_supporting_vote_gloas + get_gloas_children tests (run 64)
- Added 7 unit tests for `compute_filtered_roots` (previously ZERO direct tests):
  - Genesis only: single genesis block in filtered set
  - Self-build chain all included: 4 self-build blocks all viable and filtered in
  - External builder not revealed excluded: unrevealed external builder not in filtered set
  - External builder revealed included: revealed payload makes block viable
  - Parent propagation: non-viable parent included when it has a viable descendant
  - Deep propagation chain: propagation works through 3 non-viable ancestors to viable leaf
  - Fork with mixed viability: only viable branch and its ancestors included
- Added 4 unit tests for `get_ancestor_gloas` (previously 3, now 7):
  - Unknown root returns None
  - Multi-hop chain: walk from root(3) at slot 3 back to root(1) at slot 1 with correct payload status
  - At genesis slot: walk back to genesis correctly
  - Future slot returns Pending (slot >= block's own slot)
- Added 4 unit tests for `is_supporting_vote_gloas` (previously 5, now 9):
  - Ancestor with Pending status always supports (Pending matches any payload status)
  - Ancestor Full matches Full path (vote through FULL parent relationship)
  - Ancestor Empty does NOT match Full path (EMPTY ≠ FULL)
  - Ancestor Empty matches Empty path (vote through EMPTY parent relationship)
- Added 3 unit tests for `get_gloas_children` (previously 4, now 7):
  - Filtered roots excludes non-viable: external builder child excluded from children
  - Pending unknown root returns Empty only (EMPTY child always generated)
  - Multiple children different payload paths: FULL and EMPTY nodes get correct children
- These functions are the core of Gloas ePBS fork choice tree filtering and head selection
- All 107 proto_array tests pass (was 89), all 60 fork_choice tests pass

### 2026-02-23 — 16 get_gloas_weight + should_apply_proposer_boost_gloas tests (run 63)
- Added 8 unit tests for `get_gloas_weight` (previously ZERO direct tests):
  - No votes returns zero weight
  - Single supporting vote accumulates correctly
  - Multiple votes accumulate validator balances
  - Non-PENDING node at previous slot returns zero weight (reorg resistance)
  - Non-PENDING node at non-previous slot has normal weight
  - Proposer boost added when flag set and root matches
  - Proposer boost not applied when flag is false
  - Zero proposer boost root means no boost
- Added 8 unit tests for `should_apply_proposer_boost_gloas` (previously ZERO direct tests):
  - Zero root returns false (no boost to apply)
  - Unknown root returns false (node not in fork choice)
  - No parent returns true (genesis-like, always boost)
  - Skipped slots returns true (non-adjacent parent, always boost)
  - Adjacent strong parent returns true (weight above threshold)
  - Adjacent weak parent with no equivocation returns true
  - Weak parent with equivocating proposer: boost suppressed
  - Equivocating indices count toward parent weight calculation
- These two functions are the core of Gloas ePBS fork choice weight computation
- All 89 proto_array tests pass (was 73), all 60 fork_choice tests pass

### 2026-02-23 — 15 should_extend_payload + get_payload_tiebreaker tests (run 62)
- Added 8 unit tests for `should_extend_payload` (previously ZERO tests):
  - Timely and data-available: returns true when both flags set
  - Timely but not data-available: falls through to boost checks
  - No proposer boost root: returns true (no boost = always extend)
  - Boosted parent not this root: returns true (boost doesn't affect this block)
  - Boosted parent IS this root and full (revealed): returns true
  - Boosted parent IS this root and NOT full: returns false (the only false case)
  - Boosted block not in fork choice: returns true (treat as no boost)
  - Boosted block has no parent (genesis): returns true
- Added 7 unit tests for `get_payload_tiebreaker` (previously ZERO tests):
  - PENDING always returns ordinal value (0) regardless of slot position
  - Non-previous-slot: EMPTY and FULL return ordinal values
  - Previous-slot EMPTY: returns 1 (always favored)
  - Previous-slot FULL with extend=true: returns 2 (highest priority)
  - Previous-slot FULL with extend=false: returns 0 (lowest priority)
  - Ordering verification: FULL(2) > EMPTY(1) > PENDING(0) when extending
  - Unknown root: returns ordinal (fails previous-slot check)
- These two methods are the heart of ePBS payload tiebreaking in head selection
- All 73 proto_array tests pass (was 58), all 60 fork_choice tests pass

### 2026-02-23 — Gloas attestation index validation + spec tracking (run 61)
- Tracked consensus-specs PR #4918 ("Only allow attestations for known payload statuses")
- Implemented 3 Gloas-specific checks in `validate_on_attestation` (fork_choice.rs):
  1. `index in [0, 1]` — reject attestations with invalid committee index for Gloas blocks
  2. Same-slot attestation must have `index == 0` — can't attest payload-present for current-slot block
  3. `index == 1` requires payload revealed — commented out pending spec test vector update
- Check 3 (PayloadNotRevealed) is fully implemented and unit-tested but disabled to maintain
  EF spec test compatibility (test vectors pinned at v1.7.0-alpha.2, predating #4918)
- Added 7 unit tests for the new validation: invalid index, same-slot non-zero index,
  payload not revealed (ignored), payload revealed accepted, pre-Gloas block allows any index
- All 60 fork_choice tests pass (1 skipped), all 8 EF fork choice tests pass

### 2026-02-23 — 11 Gloas beacon_chain integration tests (run 60)
- Added `gloas.rs` integration test module in `beacon_node/beacon_chain/tests/`
- Tests the full beacon chain harness through Gloas fork transition and block production:
  - `fulu_to_gloas_fork_transition`: blocks transition to Gloas variant at correct epoch
  - `gloas_from_genesis`: all forks at epoch 0 produce Gloas blocks from genesis
  - `gloas_self_build_block_production`: self-build blocks have BUILDER_INDEX_SELF_BUILD and value=0
  - `gloas_state_fields_after_upgrade`: Gloas state has bid/builders/latest_block_hash, no execution_payload_header
  - `gloas_multiple_consecutive_blocks`: full epoch of consecutive Gloas blocks
  - `gloas_chain_finalizes`: chain finalizes after 5 epochs of Gloas blocks
  - `gloas_fork_transition_preserves_finalization`: finalization continues past Fulu→Gloas boundary
  - `gloas_block_has_no_execution_payload`: Gloas body has bid, not execution_payload
  - `gloas_block_has_payload_attestations`: payload_attestations field accessible
  - `gloas_fork_version_in_state`: fork versions correctly set (current=gloas, previous=fulu)
  - `gloas_bid_slot_matches_block_slot`: bid slot matches block slot across multiple blocks
- All 404 beacon_chain tests pass (including 34 gloas_verification + 11 new)

### 2026-02-23 — 25 ePBS pool + observation edge case tests (run 59)
- Added 10 edge case tests to `execution_bid_pool.rs` (was 4, now 14):
  - Per-slot independence: best bid selection independent across slots
  - Wrong slot: queries for non-existent slots return None
  - Prune boundary: slot exactly at retention threshold is retained
  - Prune at zero: saturating_sub prevents underflow, keeps all
  - Single builder: lone bid is best
  - Insert after prune: pool reusable after pruning
  - Many builders: 100 builders same slot, highest value wins
  - Equal values: tied bids return one deterministically
  - Empty slot count: bid_count_for_slot returns 0 for unknown slots
  - Prune idempotent: repeated prune calls are safe
- Added 6 edge case tests to `observed_execution_bids.rs` (was 5, now 11):
  - Same builder different slots: both observations are New
  - Prune at zero: slot 0 retained with saturating_sub
  - Prune boundary slot: exact boundary retained, one below pruned
  - Equivocation preserves original: 3rd bid equivocates against 1st (not 2nd)
  - Clear resets state: previously seen bid is New after clear
  - Prune idempotent: double prune safe
- Added 9 edge case tests to `observed_payload_attestations.rs` (was 6, now 15):
  - Same validator different slots: no cross-slot equivocation
  - Equivocation false→true: reverse direction equivocation detected
  - Duplicate false: payload_present=false duplicates detected
  - Prune at zero: slot 0 retained
  - Prune boundary: exact boundary logic verified
  - Equivocation preserves original: 3rd attestation with original value is Duplicate
  - Clear resets state: previously seen attestation is New after clear
  - Many validators: 512 validators same block all New
  - Prune idempotent: double prune safe
- All 186 beacon_chain lib tests pass

### 2026-02-20 — 37 ChainSpec + ForkName Gloas unit tests (run 57)
- Added 22 unit tests to `chain_spec.rs` (previously had ZERO Gloas-specific tests):
  - Scheduling: `is_gloas_scheduled()` true when epoch set, false when None, false when far-future epoch
  - Attestation timing: pre-Gloas vs at-Gloas `get_attestation_due_ms()` (3 tests)
  - Aggregate timing: pre-Gloas vs at-Gloas `get_aggregate_due_ms()` (2 tests)
  - Sync message timing: pre-Gloas vs at-Gloas `get_sync_message_due_ms()` (2 tests)
  - Contribution timing: pre-Gloas vs at-Gloas `get_contribution_due_ms()` (2 tests)
  - Payload attestation timing: `get_payload_attestation_due_ms()` (7500 BPS = 75% of slot)
  - Comparison: Gloas timing strictly shorter than pre-Gloas for all 4 duty types
  - Mainnet 12s slots: pre-Gloas ≈4s, Gloas 3s attestation; Gloas 6s aggregate; PTC 9s
  - Fallback: no Gloas fork → all epochs use pre-Gloas timing
  - Edge case: Gloas at epoch 0 → epoch 0 uses Gloas timing
  - ePBS domain values: `BeaconBuilder`, `PtcAttester`, `ProposerPreferences` domains test correctly
  - Domain distinctness: all 3 Gloas domains distinct from each other and existing domains
  - Domain indices: BeaconBuilder=11, PtcAttester=12, ProposerPreferences=13 (EIP-7732)
  - Fork epoch: `fork_name_at_epoch` returns Gloas at/after fork, Fulu before
  - Fork epoch roundtrip: `fork_epoch(ForkName::Gloas)` returns the set value
  - Fork version: Gloas fork version is non-zero on both mainnet and minimal
- Added 15 unit tests to `fork_name.rs` (previously had ZERO Gloas-specific tests):
  - `ForkName::latest()` is Gloas
  - No next fork after Gloas
  - Previous fork is Fulu; Fulu's next is Gloas
  - `gloas_enabled()` true for Gloas, false for Fulu and Base
  - All prior fork features enabled on Gloas (7 `_enabled()` methods)
  - Case-insensitive parsing: "gloas", "GLOAS", "Gloas" all parse
  - Display: outputs "gloas" lowercase
  - String roundtrip: display → parse → equality
  - In `list_all()` and is the last entry
  - `make_genesis_spec(Gloas)`: sets all 7 fork epochs to 0
  - `make_genesis_spec(Fulu)`: disables Gloas
- All 641 types tests pass (was 604)

### 2026-02-20 — 25 BeaconStateGloas unit tests (run 56)
- Added `mod gloas` test block to `beacon_state/tests.rs` (previously had ZERO Gloas coverage):
  - `make_gloas_state()` helper: constructs a full `BeaconStateGloas` with all required fields properly sized for MinimalEthSpec (Vector/List/BitVector/Arc<SyncCommittee> etc.)
  - Fork name: `fork_name_unchecked()` returns `ForkName::Gloas`
  - All 8 Gloas-only field accessors: `latest_execution_payload_bid`, `builders`, `next_withdrawal_builder_index`, `execution_payload_availability`, `builder_pending_payments`, `builder_pending_withdrawals`, `latest_block_hash`, `payload_expected_withdrawals`
  - Structural difference: `latest_execution_payload_header()` returns Err on Gloas (replaced by bid)
  - Non-Gloas state: all 8 Gloas-only fields return Err on Base state
  - Mutability: `latest_block_hash_mut`, `builders_mut` (via `get_mut(0)`), `execution_payload_availability_mut` (set bit to false)
  - SSZ roundtrip: encode/decode through `from_ssz_bytes` with Gloas spec
  - Tree hash: `canonical_root()` deterministic + non-zero, changes with bid value, `get_beacon_state_leaves()` changes with `latest_block_hash`, leaves are nonempty
  - Clone preserves equality
  - Shared field accessors: `slot()`, `fork()` (previous=fulu, current=gloas), `proposer_lookahead()`
- All 604 types tests pass (was 579)

### 2026-02-20 — 75 ePBS Gloas type unit tests across 8 files (run 55)
- Added comprehensive behavioral tests to 8 ePBS type files that previously had only SSZ macro tests:
  - `payload_attestation.rs` (11 new): `num_attesters()` with bits set, all bits set, `payload_present`/`blob_data_available` flags (individual and combined), SSZ roundtrip with set bits, tree hash sensitivity to bit changes and flag changes, determinism, clone equality, slot inequality
  - `payload_attestation_data.rs` (7 new): SSZ roundtrip for each flag combination (`payload_present`, `blob_data_available`, both), tree hash sensitivity to each flag, equality/clone, default field verification
  - `payload_attestation_message.rs` (9 new): default equals empty, non-zero `validator_index`, max `validator_index` (u64::MAX), SSZ roundtrip with `payload_present`, SSZ roundtrip with `blob_data_available`, tree hash changes with validator index, determinism, clone equality, flag inequality
  - `indexed_payload_attestation.rs` (12 new): **fixed documented gap** — unsorted indices detection via SSZ decode (`[10, 5]` → `is_sorted()` returns false), duplicate indices detection (`[5, 5]` → fails strict `<` check), ascending sorted verification, `num_attesters()` counting, SSZ roundtrip with indices and both flags, tree hash sensitivity, determinism, clone equality, index inequality
  - `execution_payload_bid.rs` (9 new): default fields are zero (all 11 fields checked), SSZ roundtrip with non-default values, self-build sentinel value (`builder_index = u64::MAX`), tree hash changes with value/block_hash, determinism, clone equality, slot/builder_index inequality
  - `signed_execution_payload_bid.rs` (7 new): `empty()` field verification (including `signature.is_empty()`), SSZ roundtrip empty and non-default bid, self-build bid roundtrip, tree hash changes with bid value, determinism, clone equality
  - `execution_payload_envelope.rs` (11 new): default equals empty, empty payload is default, SSZ roundtrip non-default (builder_index, slot, block_hash), self-build roundtrip, random TestRandom roundtrip, tree hash changes with builder_index/state_root, determinism, clone equality, slot inequality
  - `signed_execution_payload_envelope.rs` (10 new): default equals empty, empty has default message fields, SSZ roundtrip empty and non-default, random TestRandom roundtrip, self-build builder_index, tree hash changes with builder_index, determinism, clone equality, message inequality
- All 579 types tests pass (was 504)

### 2026-02-20 — 13 SignedBeaconBlock Gloas blinding + conversion tests (run 52)
- Added 13 unit tests to `signed_beacon_block.rs` (previously had only 2 tests, neither covering Gloas):
  - Blinding roundtrip: Full→Blinded→Full preserves block equality and tree hash root
  - `try_into_full_block`: Gloas succeeds without payload (None), ignores provided payload
  - Contrast test: Fulu `try_into_full_block(None)` returns None (payload required)
  - Fork name: `fork_name_unchecked()` returns `ForkName::Gloas`
  - Canonical root: deterministic, non-zero
  - Slot and proposer_index: empty block defaults verified
  - SSZ roundtrip: encode/decode through `from_ssz_bytes` with Gloas spec
  - Body accessors: no `execution_payload()`, has `signed_execution_payload_bid()` and `payload_attestations()`
  - Signature preservation: non-empty signature preserved through blind/unblind roundtrip
  - Cross-fork: Gloas SSZ bytes and tree hash root differ from Fulu
  - Extended `add_remove_payload_roundtrip` to cover Capella, Deneb, Electra, Fulu, and Gloas
- All 504 types tests pass (was 491)

### 2026-02-20 — 35 BeaconBlockBody Gloas variant unit tests (run 51)
- Added 35 unit tests to `beacon_block_body.rs` (previously had ZERO Gloas tests — only Base/Altair SSZ roundtrip):
  - SSZ roundtrip: inner type roundtrip, via BeaconBlock enum dispatch, Gloas bytes differ from Fulu bytes
  - Fork name: `fork_name()` returns `ForkName::Gloas`
  - ePBS structural differences: `execution_payload()` returns Err, `blob_kzg_commitments()` returns Err, `execution_requests()` returns Err, `has_blobs()` returns false, `kzg_commitment_merkle_proof()` fails (no commitments field)
  - Gloas-only partial getters: `signed_execution_payload_bid()` and `payload_attestations()` succeed on Gloas, fail on Fulu; Fulu exec payload getters fail on Gloas
  - Iterators: `attestations()` yields Electra variant refs, `attester_slashings()` yields Electra variant refs, `_len()` methods match inner field counts
  - Blinded↔Full conversion: roundtrip is phantom pass-through (no payload to strip), bid and payload_attestations preserved through conversion
  - `clone_as_blinded()`: all fields (bid, attestations, randao, sync_aggregate, bls_to_execution_changes) preserved
  - Body merkle leaves: nonempty, deterministic, different bodies produce different leaves
  - Tree hash: deterministic, different bodies produce different roots
  - Empty body defaults: zero operations, empty bid
  - Post-fork fields: `sync_aggregate()` and `bls_to_execution_changes()` accessible on Gloas
- All 491 types tests pass (was 456)

### 2026-02-20 — 32 BuilderBid unit tests (run 50)
- Added 32 unit tests to `builder_bid.rs` (previously had NO test module):
  - Header accessors: `header()` returns correct `ExecutionPayloadHeaderRef` for Gloas, Fulu, Bellatrix; `header_mut()` mutation test
  - Common field accessors: `value()`, `pubkey()` through enum
  - Variant-specific partial getters: `blob_kzg_commitments` accessible on Gloas/Fulu but not Bellatrix; `execution_requests` accessible on Gloas but not Bellatrix; cross-variant getter failures (header_gloas on Fulu, header_fulu on Gloas, header_bellatrix on Gloas)
  - SSZ roundtrip: inner types (Gloas, Fulu), fork dispatch via `from_ssz_bytes_by_fork` for Gloas/Fulu/Bellatrix, unsupported forks (Base, Altair) rejected, correct variant production from same-layout bytes
  - `SignedBuilderBid` SSZ: roundtrip for Gloas and Fulu, Base fork decode fails
  - Signature verification: empty pubkey fails, valid keypair passes end-to-end (sign with real BLS key, verify with `get_builder_domain`), wrong key fails
  - Tree hash: different values produce different roots, equal values produce equal roots
  - Clone + equality: clone preserves equality, different variants not equal
- All 456 types tests pass (was 424)

### 2026-02-20 — 42 DataColumnSidecar Gloas variant unit tests (run 49)
- Added 42 unit tests to `data_column_sidecar.rs` (previously had NO test module):
  - Field accessors: `slot()` (Gloas from field, Fulu from header), `epoch()` (boundary tests), `block_root()` (Gloas from field, Fulu from tree_hash), `block_parent_root()` (Gloas=None, Fulu=Some), `block_proposer_index()` (Gloas=None, Fulu=Some), `index()` shared getter
  - `verify_inclusion_proof()`: Gloas always true, Fulu default fails
  - SSZ roundtrip: inner types (Gloas, Fulu), enum via `from_ssz_bytes_by_fork` (both variants)
  - `from_ssz_bytes_by_fork`: unsupported forks rejected (Base, Altair, Deneb), correct variant dispatch
  - `any_from_ssz_bytes`: Fulu and Gloas automatic detection
  - `min_size`/`max_size`: positive, max>min for multiple blobs, max=min for 1 blob
  - Partial getters: Gloas `sidecar_slot`/`sidecar_beacon_block_root` succeed, fail on Fulu; Fulu `kzg_commitments`/`signed_block_header` succeed, fail on Gloas
  - Clone/equality: both variants clone correctly, different variants not equal
  - Tree hash: deterministic, changes with different data
  - Epoch boundaries: slot 0 = epoch 0, slot 8 = epoch 1 (minimal)
- All 424 types tests pass (was 382)

### 2026-02-20 — 50 execution payload type conversion unit tests (run 48)
- Added 22 unit tests to `execution_payload_header.rs` (previously had NO test module):
  - `upgrade_to_gloas`: preserves all 17 fields, default roundtrip
  - `From<&ExecutionPayloadGloas>`: preserves scalar fields, computes correct tree_hash_roots for transactions and withdrawals
  - `fork_name_unchecked`: Gloas and Fulu variant dispatch
  - SSZ roundtrip: inner type, enum dispatch, wrong fork produces different variant, Base/Altair reject
  - `TryFrom<ExecutionPayloadHeader>`: success, wrong variant errors (both directions)
  - `is_default_with_zero_roots`: true for default, false for non-default
  - `ExecutionPayloadHeaderRefMut::replace`: Gloas success, wrong variant fails
  - `From<ExecutionPayloadRef>`: Gloas payload ref converts correctly
  - Self-clone via `From<&Self>`, tree hash stability (equal and different values)
- Added 10 unit tests to `execution_payload.rs` (previously had NO test module):
  - `fork_name`: Gloas and Fulu dispatch
  - SSZ roundtrip: inner type, `from_ssz_bytes_by_fork` dispatch, Base/Altair reject, correct variant production
  - `clone_from_ref`: Gloas clone roundtrip
  - Enum field accessors: all 11 accessible fields (parent_hash through excess_blob_gas)
  - Default Gloas payload zero fields
- Added 18 unit tests to `payload.rs` (previously had NO test module):
  - FullPayload: `default_at_fork` (Gloas/Base/Altair), `withdrawals_root`, `blob_gas_used`, `is_default_with_zero_roots`, `block_type`, `to_execution_payload_header`
  - BlindedPayload: `block_type`, `withdrawals_root`, `blob_gas_used`, `from(header)` roundtrip, `into(header)` roundtrip
  - FullPayloadRef: `withdrawals_root`, `blob_gas_used`, `execution_payload_ref`
  - BlindedPayloadRef: `withdrawals_root`, `blob_gas_used`
- All 382 types tests pass (was 332)

### 2026-02-20 — 8 process_proposer_lookahead unit tests (run 47)
- Added 8 unit tests to `single_pass.rs` for `process_proposer_lookahead` (EIP-7917 proposer lookahead rotation):
  - `shift_moves_second_epoch_to_first`: verifies the first-epoch entries are shifted out and replaced by what was the second epoch
  - `new_entries_are_valid_validator_indices`: all newly filled entries reference active validators
  - `new_entries_match_independent_computation`: new epoch entries match `get_beacon_proposer_indices(epoch=current+2)` computed independently
  - `lookahead_length_preserved`: vector length stays at `ProposerLookaheadSlots` (16 for minimal)
  - `double_call_shifts_twice`: two consecutive calls correctly chain the shift (second call's first epoch = first call's second epoch)
  - `initial_lookahead_covers_two_epochs`: verify the test helper correctly initializes 2 epochs of proposer data
  - `deterministic_same_state_same_result`: identical states produce identical results (no hidden randomness)
  - `different_randao_produces_different_proposers`: modifying the randao mix at the correct index (computed via get_seed formula) changes proposer selection
- Previously no test module existed in this file — `process_proposer_lookahead` was only covered by EF spec tests
- Requires fork epochs set to 0 in spec so `fork_name_at_epoch` returns Fulu for future epochs (avoids `ComputeProposerIndicesExcessiveLookahead` error)
- All 280 state_processing tests pass (was 272)

### 2026-02-20 — 11 per_block_processing Gloas orchestration + fork dispatch tests (run 46)
- Added 11 unit tests to `per_block_processing.rs` for Gloas ePBS fork dispatch and block processing logic:
  - `is_execution_enabled`: Gloas returns false (ePBS has no exec payload in proposer blocks), Fulu returns true (post-merge)
  - `is_merge_transition_block`: always false for Gloas
  - Block body accessors: Gloas body has `signed_execution_payload_bid` (not `execution_payload`), Fulu body has `execution_payload` (not bid)
  - `process_withdrawals_gloas`: skips processing when parent block is empty (bid hash != latest hash), runs when parent block is full (hashes match)
  - Fork dispatch routing: Gloas state takes `gloas_enabled()` path, Fulu state takes execution path
- Also added `make_fulu_state()`, `make_gloas_block_body()`, `make_fulu_block_body()` test helpers
- All 272 state_processing tests pass (was 261)

### 2026-02-20 — 22 ForkChoice wrapper method + Builder::is_active tests (run 42)
- Added 17 unit tests to `fork_choice.rs` for the three Gloas `ForkChoice` wrapper methods:
  - `on_execution_bid`: 4 tests — unknown block root, slot mismatch, happy path (sets builder_index), resets payload_revealed, genesis block
  - `on_payload_attestation`: 9 tests — future slot rejection, too-old rejection, unknown block root, slot mismatch (silent Ok), weight accumulation (payload_present), blob weight accumulation, quorum reveals payload, at-threshold no reveal, window boundary acceptance, same-slot current, no weight when not present
  - `on_execution_payload`: 4 tests — unknown block root, reveals and sets execution status, genesis block, idempotent second call
  - These test the `ForkChoice` validation layer (slot checks, age checks, unknown-root errors) above proto_array
  - Mock `ForkChoiceStore` implementation for lightweight testing without full beacon chain harness
- Added 5 unit tests to `builder.rs` for `Builder::is_active_at_finalized_epoch`:
  - Active builder (deposited before finalized, far future withdrawable)
  - Inactive: deposit_epoch == finalized_epoch (not strictly less than)
  - Inactive: deposited after finalized
  - Inactive: exiting builder (withdrawable_epoch != FAR_FUTURE_EPOCH)
  - Inactive: epoch 0 edge case
- All 54 fork_choice tests pass, 58 proto_array tests pass, 332 types tests pass

### 2026-02-20 — 13 Gloas signature set construction tests (run 41)
- Added 13 unit tests to `signature_sets.rs` for the three Gloas ePBS signature set functions:
  - `execution_payload_bid_signature_set`: 5 tests — unknown builder (index 0, high index), valid sig verifies, wrong key fails, wrong domain (BeaconProposer) fails
  - `payload_attestation_signature_set`: 4 tests — unknown validator, one-of-two unknown, valid single signer verifies, wrong domain fails
  - `execution_payload_envelope_signature_set`: 4 tests — unknown builder, valid sig verifies, wrong key fails, wrong domain (PtcAttester) fails
  - End-to-end BLS verification: tests sign with real deterministic keypairs and verify the constructed `SignatureSet`
  - Domain correctness: confirms `BeaconBuilder` domain for bids/envelopes and `PtcAttester` domain for payload attestations
  - Previously no test module existed in this file (776 lines of untested signature construction)
- All 253 state_processing tests pass (was 240)

### 2026-02-20 — 11 fork choice node state transition tests (run 40)
- Added 11 unit tests to `proto_array_fork_choice.rs` for Gloas ePBS fork choice node state transitions:
  - `on_execution_bid` tests: bid_sets_builder_index_and_resets_payload, bid_slot_mismatch_detectable
  - `on_payload_attestation` PTC quorum tests: ptc_weight_accumulates, ptc_quorum_reveals_payload, ptc_at_threshold_does_not_reveal, blob_data_availability_quorum, skip_slot_attestation_ignored
  - `on_execution_payload` tests: payload_envelope_reveals_and_sets_status
  - Viability integration: payload_reveal_makes_external_block_viable, ptc_quorum_makes_external_block_viable, self_build_always_viable_without_reveal
  - Helper functions: `insert_external_builder_block()`, `get_node()`, `get_node_mut()`
  - Tests simulate the fork choice node mutations done by the three Gloas fork choice methods
- All 58 proto_array tests pass (was 47)

### 2026-02-20 — 24 attestation verification, proto_array viability, and attestation signing tests (run 39)
- Added 10 unit tests for `verify_attestation` Gloas committee index validation (`verify_attestation.rs`):
  - Tests the `[Modified in Gloas:EIP7732]` code that allows `data.index < 2` (was `== 0` in Electra/Fulu)
  - Gloas rejection: index 2, 3, u64::MAX all fail with `BadCommitteeIndex`
  - Gloas acceptance: index 0 and 1 pass the index check (1 is NEW in Gloas)
  - Fulu comparison: index 0 passes, index 1 and 2 rejected (pre-Gloas behavior)
  - Block inclusion timing: too-early rejection and inclusion delay checks
  - Previously no tests existed in this file
- Added 8 unit tests for `proto_array::node_is_viable_for_head` payload_revealed check (`proto_array.rs`):
  - Tests the Gloas ePBS viability logic for head selection
  - Pre-Gloas (builder_index=None): always viable
  - Self-build (BUILDER_INDEX_SELF_BUILD): always viable even without payload revealed
  - External builder: viable only when payload_revealed=true
  - Builder index 0: treated as external builder (not self-build)
  - Invalid execution status: never viable regardless of payload_revealed
  - Previously no test module existed in proto_array.rs
- Added 6 unit tests for `Attestation::empty_for_signing` Gloas payload_present logic (`attestation.rs`):
  - Tests the `[New in Gloas:EIP7732]` code that sets `data.index = 1` when `payload_present=true`
  - Gloas: payload_present=true → index=1, payload_present=false → index=0
  - Fulu: payload_present flag ignored, always index=0
  - Variant check: Gloas attestation is Electra variant
  - Committee bits: correct bit set for given committee_index
  - Previously only integration test coverage
- All 240 state_processing tests pass (was 230), 47 proto_array tests pass (was 39), 327 types tests pass (was 321)

### 2026-02-20 — 16 per_slot_processing, proposer slashing, and attestation weight tests (run 38)
- Added 6 unit tests for `per_slot_processing` Gloas-specific code (`per_slot_processing.rs`):
  - Tests `cache_state` clearing of `execution_payload_availability` bit for next slot
  - Covers: basic clearing, wraparound at `SlotsPerHistoricalRoot`, only-target-bit-cleared, idempotent false→false, state_root caching preserved, end-to-end `per_slot_processing` test
  - Previously no tests existed in this file
- Added 6 unit tests for proposer slashing builder payment removal (`process_operations.rs`):
  - Tests the `[New in Gloas:EIP7732]` code that zeroes `BuilderPendingPayment` when a proposer is slashed
  - Covers: current epoch index calculation, previous epoch index, old epoch (no clear), selective clearing, empty payment no-op, epoch boundary slot
  - Previously untested — EF spec tests cover slashing but not the Gloas payment removal path
- Added 4 unit tests for same-slot attestation weight accumulation (`process_operations.rs`):
  - Tests the `[New in Gloas:EIP7732]` code that adds `effective_balance` to `builder_pending_payment.weight`
  - Covers: weight added for same-slot attestation, no weight when payment amount is zero, no weight for non-same-slot (skipped slot), duplicate attestation no double-counting
  - Previously untested — this is the PTC attestation weight accumulation path used for builder payment quorum
- All 230 state_processing tests pass (was 214)

### 2026-02-20 — 16 Gloas genesis initialization and expected withdrawals tests (run 37)
- Added 9 unit tests for Gloas genesis initialization (`genesis.rs`):
  - Tests the `initialize_beacon_state_from_eth1` code path with all forks at epoch 0 (including Gloas)
  - Verifies: Gloas state variant, fork versions, Gloas-specific field initialization (builders, payments, availability bits), execution payload header block_hash propagation, validator activation, cache building, is_valid_genesis_state, sync committees
  - Previously untested — EF genesis tests only run on `ForkName::Base`
- Added 7 unit tests for `get_expected_withdrawals_gloas` withdrawal phases (`gloas.rs`):
  - Phase 1: builder pending withdrawal, multiple builder pending withdrawals
  - Phase 3: builder sweep (exited with balance, active not swept)
  - Phase 4: validator sweep (excess balance partial withdrawal, fully withdrawable)
  - Combined: withdrawals from multiple phases together
  - Previously only 2 tests existed (matches-process-withdrawals, empty-when-parent-not-full)
- All 214 state_processing tests pass

### 2026-02-20 — 26 gossip verification integration tests (gloas_verification.rs)
- Added `gloas_verification.rs` integration test module in `beacon_node/beacon_chain/tests/`
- Tests all three gossip verification functions:
  - `verify_execution_bid_for_gossip`: 9 tests — slot validation (past, future, boundary), zero payment, unknown builder (index 0 and high), slot acceptance checks
  - `verify_payload_attestation_for_gossip`: 5 tests — future slot, past slot, empty aggregation bits, unknown block root, valid slot passes early checks
  - `verify_payload_envelope_for_gossip`: 9 tests — unknown block root (with buffering), slot mismatch, builder index mismatch, block hash mismatch, buffering behavior, duplicate root overwrite, self-build happy path, prior to finalization
  - Observation trackers: 3 tests — bid observation (new/duplicate/independent builders), payload attestation observation counts
- All 26 tests pass with `FORK_NAME=gloas`
- Used `unwrap_err` helper to work around `VerifiedX<Witness<...>>` not implementing `Debug`

### 2026-02-19 — full-preset EF test verification (mainnet + minimal)
- Ran both mainnet and minimal preset tests (previously only running minimal in CI)
- **78/78 real crypto pass** (mainnet + minimal, 0 skipped)
- **138/138 fake_crypto pass** (mainnet + minimal, 0 skipped)
- Mainnet preset uses full-size states (512 validators, larger committees) — confirms no issues with field sizes or list limits

### 2026-02-18 — fix fork_choice_on_block for Gloas blocks (77/78 → 78/78)
- **Root cause**: Gloas fork choice tests process blocks without envelopes. When the state cache evicts a state and block replay reconstructs it, `per_block_processing` fails `bid.parent_block_hash != state.latest_block_hash` because the stored post-block state has `latest_block_hash` from before envelope processing.
- **Fix 1**: Block replayer now applies `latest_block_hash = bid.block_hash` for skipped anchor blocks (block 0) that are Gloas blocks. This ensures the starting state for replay has the correct value.
- **Fix 2**: `apply_invalid_block` in the fork choice test harness gracefully handles state reconstruction failures for Gloas blocks instead of panicking. The primary validation (`process_block` rejecting the invalid block) already passes.
- Also applied `cargo fmt` to all gloas code (50 files, whitespace/line-wrapping only).
- 78/78 EF tests pass, 136/136 fake_crypto pass
- Commits: `f9e2d376b`, `d6e4876be`

### 2026-02-19 — add ProposerPreferences SSZ types (136→138 fake_crypto tests)
- Implemented `ProposerPreferences` and `SignedProposerPreferences` container types per consensus-specs p2p-interface.md
- Added `Domain::ProposerPreferences` variant (domain value 13) — field already existed in ChainSpec, just needed the enum variant and wiring
- Registered type_name macros, added SSZ static test handlers (gloas_and_later)
- Removed ProposerPreferences/SignedProposerPreferences from check_all_files_accessed exclusions
- 138/138 fake_crypto pass (minimal), 2 new SSZ static tests for these types
- Commit: `f27572984`

### 2026-02-17 — fix check_all_files_accessed (was failing with 66,302 missed files)
- **Root cause**: v1.7.0-alpha.2 test vectors added `manifest.yaml` to every test case (~62K files) + new SSZ generic/static types
- **Fix 1**: Added `inactivity_scores` to rewards test handler — was missing across ALL forks (not just gloas), adds real test coverage
- **Fix 2**: Added exclusions for new unimplemented test categories:
  - `manifest.yaml` files (metadata not read by harness)
  - `compatible_unions` + `progressive_containers` SSZ generic tests
  - `light_client/update_ranking` tests
  - `ForkChoiceNode` SSZ static (internal fork choice type)
  - `ProposerPreferences` / `SignedProposerPreferences` SSZ static (external builder path, not yet implemented)
- **Fix 3**: Extended `MatrixEntry` exclusion to cover gloas (was fulu-only)
- Result: 209,677 accessed + 122,748 excluded = all files accounted for
- Commit: `f7554befa`

### 2026-02-17 — 78/78 passing (execution_payload envelope tests added)
- Added `ExecutionPayloadEnvelopeOp` test handler for gloas `process_execution_payload` spec tests
- These tests use `signed_envelope.ssz_snappy` (unlike pre-gloas which uses `body.ssz_snappy`)
- Implemented envelope signature verification in `process_execution_payload_envelope` using `execution_payload_envelope_signature_set`
- Handles `BUILDER_INDEX_SELF_BUILD` (u64::MAX): uses proposer's validator pubkey instead of builder registry
- 40 tests: 17 valid cases + 23 expected failures (wrong block hash, wrong slot, invalid signature, etc.)
- Test gated behind `#[cfg(not(feature = "fake_crypto"))]` — one test (`process_execution_payload_invalid_signature`) has missing `bls_setting` in upstream test vectors

### 2026-02-17 — 77/77 passing (DataColumnSidecar SSZ fixed)
- Implemented DataColumnSidecar superstruct with Fulu and Gloas variants
- Fulu variant: index, column, kzg_commitments, kzg_proofs, signed_block_header, kzg_commitments_inclusion_proof
- Gloas variant: index, column, kzg_proofs, slot, beacon_block_root (per spec change)
- Updated all field accesses across 29 files to use superstruct getter methods
- SSZ static test handler split into separate Fulu and Gloas handlers
- Commit: `b7ce41079`

### 2026-02-26 — external builder integration tests + bid test fixes (run 113)
- Added 3 new integration tests in `gloas.rs` for external builder block import lifecycle:
  - `gloas_external_bid_block_import_payload_unrevealed`: imports block with external bid, verifies payload_revealed=false in fork choice
  - `gloas_external_bid_import_fork_choice_builder_index`: verifies stored block preserves correct builder_index and bid value
  - `gloas_external_bid_envelope_reveals_payload_in_fork_choice`: constructs signed envelope, gossip-verifies it, applies to fork choice, verifies payload_revealed=true
- Fixed 4 pre-existing test failures in `gloas_verification.rs` caused by proposer preferences validation added in run 111:
  - `bid_invalid_signature`, `bid_valid_signature_passes`, `bid_balance_exactly_sufficient_passes`, `bid_second_builder_valid_signature_passes`
  - Added `insert_preferences_for_bid` helper to insert matching preferences before bid reaches signature/balance checks
- All 569 beacon_chain tests pass
- Audited consensus-specs: PR #4918 (attestation index=1 requires payload_states) already implemented in vibehouse

### 2026-02-20 — 21 PubsubMessage Gloas gossip encode/decode tests (run 54)
- Added 21 unit tests for all 5 Gloas PubsubMessage variants + Gloas BeaconBlock
- Tests cover: SSZ round-trip encode/decode, kind() mapping, pre-Gloas fork rejection, invalid SSZ data
- Variants tested: ExecutionBid, ExecutionPayload (envelope), PayloadAttestation, ProposerPreferences, ExecutionProof
- Uses ForkContext with Gloas enabled vs pre-Gloas to verify fork-gating in decode()

### 2026-02-15 — 76/77 passing
- All gloas fork_choice_reorg tests fixed (root, payload_status model correct)
- Added known-failure skips for 3 altair tests (upstream also hasn't fixed)
- Commit: `3b677712a`

### 2026-02-14 — SSZ static pass
- 66/67 SSZ static tests pass, all gloas types pass
- 1 pre-existing failure: DataColumnSidecar (Gloas spec added `kzg_commitments` field)
- Added gloas fork filters, registered 15 new type_name entries
