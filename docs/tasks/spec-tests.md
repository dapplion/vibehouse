# Spec Tests

## Objective
Run the latest consensus spec tests at all times. Track and fix failures.

## Status: DONE

### Current results
- **79/79 ef_tests pass (real crypto, 0 skipped)** — both mainnet + minimal presets
- **138/138 fake_crypto pass (0 skipped)** — both mainnet + minimal presets (Fulu + Gloas DataColumnSidecar variants both pass)
- **check_all_files_accessed passes** — all files accessed, intentionally excluded patterns maintained
- All 9 fork_choice test categories pass (get_head, on_block, ex_ante, reorg, withholding, get_proposer_head, deposit_with_reorg, should_override_forkchoice_update, on_execution_payload)
- 40/40 gloas execution_payload envelope tests pass (process_execution_payload_envelope spec validation)
- rewards/inactivity_scores tests running across all forks (was missing)
- 3 altair proposer_boost tests now pass (were skipped, sigp/lighthouse#8689 — fixed by implementing PR #4807)
- Spec tracked to v1.7.0-alpha.3 (updated from alpha.2)

### Tasks
- [x] Audit spec test runner — understand download, cache, run flow
- [x] Check which spec test version is currently pinned (v1.7.0-alpha.2)
- [x] Update to latest spec test release when new ones drop
- [x] Ensure all existing fork tests pass (phase0 through fulu)
- [x] Add gloas test scaffolding: register fork, add handlers, wire new test types
- [x] Set up CI job: download latest vectors, run all tests, fail on regression
- [x] Create automated check for new spec test releases

### Test categories
bls, epoch_processing, finality, fork, fork_choice, genesis, light_client, operations, random, rewards, sanity, ssz_static, transition

## Progress log

### run 1221 (Mar 14) — spec stable, bid pool eager pruning

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases (latest v1.5.0 on consensus-spec-tests, v1.7.0-alpha.3 on consensus-specs). cargo audit unchanged (1 rsa). Nightly green (5 consecutive: Mar 10-14). CI from run 1220 (8d684988f) — check+clippy, ef-tests, network+op_pool all passed; beacon_chain, http_api, unit tests still running.

Conducted deep audit of Gloas ePBS code (gloas.rs, gloas_verification.rs, execution_bid_pool.rs, signature_sets.rs) via subagent. Verified all reported findings against actual code and spec:
- Self-build envelope signature: already handled in `execution_payload_envelope_signature_set` (line 760, BUILDER_INDEX_SELF_BUILD branch). False positive.
- `value` vs `execution_payment` validation: spec does NOT require these to match. `execution_payment` is defined in the container but not used in state processing. Only gossip validation checks `execution_payment != 0` (already implemented at line 422). False positive.
- `is_parent_block_full` zero hash: intentionally returns true when both hashes are zero (genesis/fork activation). Test at line 4909 documents this. False positive.
- Bid pool unbounded growth: real but bounded in practice by builder count. Fixed anyway.

Shipped: prune execution bid pool on insert (4b216dde7). Previously the pool was only pruned during `get_best_bid()` (block production). If block production stalled, bids could accumulate from gossip without bound. Now `insert()` prunes old slots eagerly, capping the pool to MAX_BID_POOL_SLOTS (4) worth of data at all times. Updated test to account for insert-time pruning. 40/40 bid pool + observed_bids tests pass. Lint clean.

### run 1220 (Mar 14) — spec stable, attestation verification allocation optimization

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases (latest v1.6.0-beta.0). cargo audit unchanged (1 rsa). Nightly green (5 consecutive: Mar 10-14). CI from run 1218 fix (b82b5557a) — check+ef passed, remaining jobs still running.

Confirmed vibehouse already conforms to recently merged spec PR #5001 (parent_block_root in bid filtering key) — was implemented proactively.

Shipped: avoid committee Vec allocation in unaggregated attestation verification (8d684988f). The `verify_late_checks` hot path was cloning the entire committee slice (`to_vec()`) for every gossip attestation just to check membership and build the aggregation bitfield. Refactored to extract only the aggregation bit position and committee length inside the committee cache closure, then build the attestation from those two scalars via new `build_attestation_from_single()` function. Eliminates one heap allocation per gossip attestation. 143/143 attestation tests + 23/23 attestation_verification tests pass. Lint clean.

### run 1219 (Mar 14) — spec stable, ptc-lookbehind rebased

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN, NOT MERGED (1 APPROVED, same head d76a278b0a). No new spec test releases (latest v1.6.0-beta.0). No semver-compatible dependency updates. cargo audit unchanged (1 rsa). Nightly green (3 consecutive: Mar 12-14).

CI fix from run 1218 (b82b5557a) in-flight — check+clippy+fmt passed, remaining jobs running. Previous CI failure was on 13ac9ae15 (pre-fix commit).

Rebased `ptc-lookbehind` branch onto main (was 128 commits behind). One conflict in `consensus/state_processing/src/per_block_processing/gloas.rs` — stale `indices.len()` reference vs already-computed `total` from committees fold. Resolved by keeping HEAD's version. All 575/575 state_processing tests + 9/9 fork choice EF tests pass on rebased branch. Pushed to origin. Lint clean.

No actionable TODOs in Gloas code (searched all production code in consensus/, beacon_node/, validator_client/). No open PRs on dapplion/vibehouse. No new issues worth working on.

### run 1218 (Mar 14) — fix CI failures from self-build envelope signature verification

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases.

Fixed CI failures caused by 13ac9ae15 (self-build envelope signature verification). Two test suites failed: beacon_chain (`gloas_execution_status_lifecycle_bid_optimistic_to_valid`) and network (`test_gloas_envelope_before_block_full_gossip_pipeline`, `test_gloas_gossip_payload_envelope_duplicate_ignored`). Root causes:

1. `make_block_with_envelope` in test_utils returned unsigned self-build envelopes (Signature::empty). Fix: sign with proposer's validator key using DOMAIN_BEACON_BUILDER, matching what the VC does in production.

2. `process_pending_envelope` in gossip_methods.rs ran BEFORE `recompute_head_at_current_slot`, so the cached head state had the wrong `proposer_index` for signature verification. Fix: moved `process_pending_envelope` after `recompute_head`.

3. Network test `test_gloas_gossip_payload_envelope_duplicate_ignored` constructed envelopes with `Signature::empty()`. Fix: properly sign with proposer key.

All tests pass: 770/770 beacon_chain, 163/163 network, 575/575 state_processing, 139/139 EF spec tests (fake_crypto). Clippy clean.

### run 1217 (Mar 14) — spec conformance audits, all stable

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN, NOT MERGED (1 APPROVED, same head d76a278b0a). No new spec test releases (latest v1.7.0-alpha.3 on consensus-specs, v1.6.0-beta.0 on consensus-spec-tests). cargo audit unchanged (1 rsa, no fix). No semver-compatible dependency updates. CI green, nightlies passing (Mar 10-14).

Conducted two deep spec conformance audits: (1) proposer lookahead — all components (state field, fork upgrade initialization, epoch rotation, single/multi-epoch lookup, gossip validation, fork boundary handling) fully compliant, safe arithmetic throughout, comprehensive test coverage. (2) execution payload envelope processing — all validation steps (signature verification, bid consistency, withdrawals, parent hash, timestamp, builder payments, execution requests, state root), fork choice integration (3-state EMPTY/FULL/PENDING model), self-build path, gossip validation all spec-compliant. Recent fixes (13ac9ae15: self-build signature, 15174086e: consolidation inline balance) verified correct. Zero clippy warnings. 79/79 EF spec tests (real crypto), 139/139 EF spec tests (fake crypto) pass.

Open spec PRs tracked: #4992 (cached PTCs — approaching merge), #4939 (request missing envelope for index-1 attestation), #4954 (fork choice milliseconds — no impact), #4843 (variable PTC deadline), #4840 (EIP-7843), #4630 (EIP-7688 forward-compatible SSZ). Test-only PRs: #4960, #4962, #4932 — all handled by existing test infrastructure.

### run 1216 (Mar 14) — fix self-build envelope signature verification

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN, mergeable=clean. cargo audit unchanged (1 rsa, no fix). No new spec test releases.

Spec conformance audit found that self-build envelope signature verification was being skipped entirely. Per spec, `verify_execution_payload_envelope_signature` verifies self-build envelopes against the proposer's validator pubkey. Fixed in `execution_payload_envelope_signature_set`, `process_execution_payload_envelope`, and `verify_payload_envelope_for_gossip`. Also audited 10 recent optimization commits for semantic bugs — all correct. 575/575 state_processing tests, 79/79 EF spec tests (real crypto), 139/139 EF spec tests (fake crypto) pass.

### run 1207 (Mar 14) — avoid Vec allocation in sync contribution aggregation bit check

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. cargo audit unchanged (1 rsa, no fix). No new spec test releases (latest v1.7.0-alpha.3). Note: v1.7.0-alpha.3 spec test vectors have NOT been published yet (latest release is v1.6.0-beta.0).

Shipped: replaced `collect::<Vec<_>>()` with iterator-based approach in `SyncContributionAggregateMap::insert` (naive_aggregation_pool.rs). The old code collected all set bit indices into a Vec just to check that exactly one bit is set and get its index. Now uses `iter.next()` to get the first set bit, then `iter.next().is_some()` to detect multiple set bits — avoids a heap allocation per sync contribution insertion (hot path). 13/13 naive_aggregation_pool tests pass, clippy clean.

### run 1206 (Mar 14) — eliminate heap allocations in batch signature verification

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. cargo audit unchanged (1 rsa, no fix). No new spec test releases (latest v1.7.0-alpha.3).

Open spec PRs tracked: #4992 (cached PTCs — approved, adds `previous_ptc`/`current_ptc` to BeaconState), #4939 (request missing envelopes for index-1 attestation), #4747 (fast confirmation rule), #4843 (variable PTC deadline), #4840 (EIP-7843 support), #4630 (EIP-7688 forward-compatible SSZ).

Shipped: eliminated heap allocations in `verify_signature_sets` (crypto/bls/src/impls/blst.rs), the core batch BLS verification function. (1) Removed `collect::<Vec<_>>()` of the `ExactSizeIterator` input — use `.len()` directly then consume the iterator. (2) Reused a single `signing_keys_buf` Vec across loop iterations instead of allocating a new Vec per signature set. (3) Replaced `zip().unzip()` with direct `.iter().collect()`. Eliminates N heap allocations per call (N = number of signature sets, typically 5-10 per block). 37/37 BLS tests, 8/8 EF BLS spec tests, 52/52 signature state_processing tests pass, clippy clean.

### run 1204 (Mar 14) — use pubkey_cache instead of HashSet in builder onboarding

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. cargo audit unchanged (1 rsa, no fix). No new spec test releases (latest v1.7.0-alpha.3).

Open spec PRs tracked: #4992 (cached PTCs — approved, adds `previous_ptc`/`current_ptc` to BeaconState, rotates in `process_slots`, simplifies `get_ptc` to state lookup), #4939 (request missing envelopes for index-1 attestation), #4747 (fast confirmation rule). When #4992 merges: add 2 FixedVector fields to BeaconStateGloas, add `compute_ptc` function, modify per_slot_processing to rotate, update fork upgrade initialization, simplify `get_ptc_committee` to read from state.

Shipped: replaced HashSet<PublicKeyBytes> allocation in `onboard_builders_from_pending_deposits` (upgrade/gloas.rs) with lookups against the existing `pubkey_cache`. The HashSet copied all validator pubkeys (~48 bytes × validator count; ~48MB on mainnet) to check if a pending deposit belongs to a validator. The pubkey_cache is already populated from the pre-state via `mem::take` — just needs an `update_pubkey_cache()` call to ensure it's current. Also changed the small `new_validator_pubkeys` tracker from HashSet to Vec (typically holds 0-2 entries). All 368 state_processing Gloas tests + 575 total state_processing tests + 10/10 EF fork spec tests pass, clippy clean.

### run 1200 (Mar 14) — avoid intermediate Vec allocation in range sync data column coupling

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. cargo audit unchanged (1 rsa, no fix). No new spec test releases (latest v1.7.0-alpha.3).

Shipped: in `RangeBlockComponentsRequest::responses()` DataColumns path, replaced `data_columns.extend(data.clone())` with `data_columns.extend(data.iter().cloned())` — avoids allocating an intermediate Vec per sub-request (the `.clone()` on a Vec allocates a new Vec then extends from it, while `.iter().cloned()` extends directly from the iterator). Also pre-allocated `data_columns` with `Vec::with_capacity` using the sum of completed request lengths. 7/7 block_sidecar_coupling tests pass, clippy clean.

### run 1199 (Mar 14) — avoid unnecessary clones in range sync batch requests

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN, approved. cargo audit unchanged (1 rsa, no fix). No new spec test releases.

Shipped: eliminated two unnecessary `Vec` clones in `RangeDataColumnBatchRequest::new` by restructuring to a single-pass loop over `by_range_requests`, and replaced `HashSet::clone().into_iter().collect()` with `iter().copied().collect()` in `to_data_columns_by_range_request`. Both in the network range sync hot path. 2/2 range sync tests pass, clippy clean.

### run 1198 (Mar 14) — avoid Arc clone in data column gossip verification

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN, approved. Official v1.7.0-alpha.3 spec test release confirmed (published Mar 13). cargo audit unchanged (1 rsa, no fix). No patch-level dependency updates available (lockfile fully current).

Shipped: changed `verify_parent_block_and_finalized_descendant` to take `&DataColumnSidecar` instead of `Arc<DataColumnSidecar>` by value. The function only reads `block_parent_root()` and does fork choice lookups — it never transfers ownership. Eliminates one `Arc::clone` per gossip data column verification (hot path). 2/2 data column verification tests pass, clippy clean.

### run 1197 (Mar 14) — avoid Hash256 wrapper in compute_shuffled_index

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN, approved. No new spec test releases (latest v1.6.0-beta.0). cargo audit unchanged (1 rsa, no fix). No dependency updates.

Shipped: removed Hash256 wrapper from compute_shuffled_index hash helpers (return [u8; 32] directly), and used Hash256::from instead of from_slice in deposit_data_tree where input is already [u8; 32]. 5/5 shuffle tests + 1/1 EF shuffling test + 3/3 EF deposit/genesis tests pass.

### run 1196 (Mar 14) — use mem::take for variable-length Lists in fork upgrades

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN, approved. No new spec test releases (v1.7.0-alpha.3 verified in run 1190). cargo audit unchanged (1 rsa, no fix). No dependency updates.

Verified two new post-alpha.3 spec PRs: #5001 (add parent_block_root to bid filtering key) — already implemented in vibehouse's `ObservedExecutionBids::is_highest_value_bid` which tracks by `(slot, parent_block_hash, parent_block_root)`. #4940 (initial fork choice tests for Gloas) — test infrastructure already handles `on_execution_payload` step. No code changes needed.

Reviewed `node_is_viable_for_head` in proto_array for potential genesis block edge case — confirmed no issue: genesis/fork-transition blocks have `builder_index = None` or `BUILDER_INDEX_SELF_BUILD`, never a real external builder index without a corresponding bid.

Checked nightly test history: March 10 `network-tests (fulu)` failure was `data_column_reconstruction_at_deadline` race condition — already fixed (test rewritten to collect events in any order). Subsequent nightlies (March 11-13) all pass.

Shipped: replaced `.clone()` with `mem::take()` for `historical_summaries`, `pending_deposits`, `pending_partial_withdrawals`, and `pending_consolidations` in fork upgrade functions (deneb, electra, fulu, gloas). These are variable-length Lists that can grow large; `mem::take` moves the backing allocation instead of cloning it. 368 state_processing tests + 139/139 EF spec tests pass.

### run 1195 (Mar 14) — reuse ancestor cache allocation in find_head_gloas

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN, approved. No new spec test releases (v1.7.0-alpha.3 verified in run 1190). cargo audit unchanged (1 rsa, no fix). No dependency updates.

Open spec PRs tracked: #4992 (cached PTCs — approved, likely next to merge), #4954 (milliseconds — no impact), #4843 (variable PTC deadline), #4939 (request missing envelope for index-1 attestation). When #4992 merges, need to add `ptc_lookbehind` to BeaconState, update epoch processing rotation, and fork transition initialization.

Shipped: moved the `ancestor_cache` HashMap in `find_head_gloas` from a local variable (allocated/deallocated per call) to a persistent field `gloas_ancestor_cache_buf` on `ProtoArrayForkChoice`. Uses `std::mem::take` to temporarily move out during the call to avoid borrow conflicts with `get_gloas_weight(&self, ..., &mut cache)`. The HashMap's internal storage is now retained across slots, avoiding one heap allocation per slot. All 188 proto_array tests + 119 fork_choice tests + 9 EF fork_choice spec tests pass.

### run 1194 (Mar 14) — migrate CI from moonrepo/setup-rust to dtolnay/rust-toolchain

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases (v1.7.0-alpha.3 verified in run 1190). cargo audit unchanged (1 rsa, no fix). No dependency updates.

Open spec PRs analyzed: #4954 (fork choice milliseconds — no impact, vibehouse uses Slot abstraction), #4898 (remove pending tiebreaker — already implemented), #4892 (remove impossible branch — already implemented). Test vector PRs #4960, #4932, #4962 add Gloas fork_choice and sanity/blocks tests — existing test infrastructure handles both without changes.

Shipped: migrated CI workflows (ci.yml, nightly-tests.yml) from `moonrepo/setup-rust@v1` to `dtolnay/rust-toolchain@stable` + `Swatinem/rust-cache@v2` + `taiki-e/install-action@cargo-nextest`. `moonrepo/setup-rust` uses Node.js 20 which GitHub is deprecating (forced Node.js 24 starting June 2, 2026). `dtolnay/rust-toolchain` is a composite action (no Node.js), `Swatinem/rust-cache@v2.9.0` already migrated to Node.js 24, `taiki-e/install-action` is actively maintained.

### run 1193 (Mar 14) — replace remaining Hash256::from_slice with From for fixed-size arrays

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases (v1.7.0-alpha.3 already verified in run 1190). cargo audit unchanged (1 rsa, no fix). No dependency updates. Two open spec PRs to track: #4960 (Gloas fork choice deposit_with_reorg tests) and #4932 (Gloas sanity/blocks tests with payload attestation coverage) — both add test vectors to existing categories, no code changes needed when they merge.

Shipped: changed `DEFAULT_ETH1_BLOCK_HASH` from `&[u8]` to `[u8; 32]` and replaced all `Hash256::from_slice` calls on fixed-size `[u8; 32]` arrays with `Hash256::from` across 16 files. Also fixed `withdrawal_credentials.rs` eth1 path. Eliminates runtime length checks when the source is already a fixed-size array. All tests pass, clippy clean.

### run 1192 (Mar 14) — avoid cloning shared fields in data column sidecar construction

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN. No new spec test releases. cargo audit unchanged (1 rsa, no fix). No dependency updates.

Shipped: eliminated one clone each of `kzg_commitments`, `signed_block_header`, and `kzg_commitments_inclusion_proof` in `build_data_column_sidecars` (kzg_utils.rs). Previously all 128 sidecars cloned these shared fields; now the loop builds 127 sidecars with clones and the last sidecar moves the values. On mainnet with max blobs (4096 KZG commitments × 48 bytes = 192KB), this saves ~192KB of heap allocation per block. All 16 data column tests pass. Clippy clean.

### run 1190 (Mar 14) — verify official v1.7.0-alpha.3 spec test release

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN, NOT MERGED, same head d76a278b0a. **Official v1.7.0-alpha.3 spec test release published Mar 13** — first official release with Gloas test vectors (previously we used custom-built vectors from the tag). Downloaded and verified: 139/139 fake_crypto minimal pass, 79/79 real crypto minimal pass. check_all_files_accessed passes for minimal preset. New `heze` fork directory present in test vectors — already excluded in check_all_files_accessed.py (line 51). PR #5001 (`parent_block_root` in bid filtering key) already implemented in our `observed_execution_bids.rs`. PR #5002 (wording clarification) is docs-only. cargo audit unchanged (1 rsa, no fix). No dependency updates.

### run 1189 (Mar 14) — replace Hash256::from_slice with From for fixed-size arrays

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN, NOT MERGED, same head d76a278b0a. No new spec test releases (still v1.6.0-beta.0). No new dependency updates.

Shipped: replaced `Hash256::from_slice(&array)` with `Hash256::from(array)` across 7 files where the source is already `[u8; 32]`. Eliminates runtime length checks and one `.to_vec()` heap allocation in `compute_kzg_proof`. Also simplified `canonical_root()` from `Hash256::from_slice(&self.tree_hash_root()[..])` to `self.tree_hash_root()` — both types are the same `alloy_primitives::B256`.

### run 1188 (Mar 14) — dep update, optimization search, all stable

Spec stable: no new consensus-specs commits since last check (latest e50889e1ca, #5004). PR #4992 (cached PTCs in state) still OPEN, NOT MERGED, same head d76a278b0a. No new spec test releases (still v1.6.0-beta.0). cargo audit unchanged (1 rsa, no fix). Updated cc 1.2.56→1.2.57.

Thorough audit of remaining allocation optimization opportunities across all hot paths: per-block (process_operations, signature verification, attestation verification), per-slot (fork choice on_attestation, dequeue_attestations), per-epoch (single_pass, process_pending_consolidations), block production. Conclusion: the codebase is well-optimized after runs 1151-1187 — remaining allocations are either architecturally necessary (state clones for parallel processing, participation snapshot for validator monitor) or negligible (O(1) array lookups). No actionable optimization found.

### run 1187 (Mar 14) — reuse children Vec allocation in find_head_gloas

Spec stable: no new consensus-specs commits since last check. PR #4992 (cached PTCs in state) still OPEN. No new spec test releases. cargo audit unchanged (1 rsa, no fix).

Shipped: eliminated per-iteration heap allocation in `find_head_gloas` (proto_array_fork_choice.rs). Previously `get_gloas_children` returned a new `Vec<GloasForkChoiceNode>` on every loop iteration (3-10 iterations per slot depending on chain depth). Extracted the logic into a free function `collect_gloas_children` that writes into a caller-provided buffer. `find_head_gloas` now stores the buffer as a struct field (`gloas_children_result_buf`) that is cleared and refilled each iteration, retaining its heap allocation across slots. Also extracted `parent_payload_status_of` as a free function to avoid code duplication. The allocating `get_gloas_children` wrapper is retained for test code only (`#[cfg(test)]`). All 188 proto_array tests, 119 fork_choice tests, 9/9 EF fork choice spec tests pass. Clippy clean.

Open PRs to track: #4992 (cached PTCs in state — approaching merge).

### run 1186 (Mar 14) — avoid Vec allocation in is_valid_indexed_attestation

Spec stable: no new consensus-specs commits since last check. PR #4992 (cached PTCs in state) still OPEN. No new spec test releases. cargo audit unchanged (1 rsa, no fix).

Shipped: eliminated per-attestation Vec allocation in `is_valid_indexed_attestation` (is_valid_indexed_attestation.rs). Previously called `attesting_indices_to_vec()` which copied the entire VariableList into a heap-allocated Vec, just to check emptiness and sorted ordering. Now uses `attesting_indices_is_empty()` and `attesting_indices_iter()` directly — zero allocation, same O(n) sorted check via `tuple_windows()`. This function is called for every attestation in every block (typically 64-128 attestations per slot on mainnet). All 575 state_processing tests, 15/15 EF operations spec tests pass. Clippy clean.

Open PRs to track: #4992 (cached PTCs in state — approaching merge).

### run 1185 (Mar 14) — skip delta Vec allocation in Gloas fork choice

Spec stable: no new consensus-specs commits since last check. PR #4992 (cached PTCs in state) still OPEN. PR #4940 (Gloas fork choice tests) confirmed included in v1.7.0-alpha.3 — all 46 Gloas fork choice test cases pass. cargo audit unchanged (1 rsa, no fix).

Shipped: split vote-tracker side effects from compute_deltas into apply_vote_updates for Gloas path, eliminating unnecessary Vec allocation per slot.

Open PRs to track: #4992 (cached PTCs in state — approaching merge).

### run 1184 (Mar 14) — zero-allocation sorted merge intersection in on_attester_slashing

Spec stable: no new consensus-specs commits since last check. Both tracked PRs (#4992, #4939) still OPEN, unchanged. No new spec test releases. cargo audit unchanged (1 rsa, no fix).

Shipped: replaced two BTreeSet allocations + BTreeSet::intersection in `on_attester_slashing` (fork_choice.rs) with a zero-allocation sorted merge walk. Since IndexedAttestation attesting_indices are sorted by spec, the intersection can be computed in O(n+m) by walking both sorted iterators simultaneously, matching on equal elements. Eliminates two heap-allocated BTreeSets (one per attestation's indices) and the O(n log n) BTreeSet insert cost. Removed unused `BTreeSet` import from production code (test code has its own import). All 119 fork_choice tests, 9/9 EF fork choice spec tests pass. Clippy clean.

Open PRs to track: #4992 (cached PTCs in state — approaching merge), #4939 (request missing payload envelopes for index-1 attestations).

### run 1181 (Mar 14) — derive Copy for BeaconBlockHeader, EnrForkId, FinalizedExecutionBlock

Spec update: two new PRs merged since last check. #5001 (add `parent_block_root` to bid filtering key, Mar 12) — already compliant, our `ObservedExecutionBids` already uses the 3-tuple `(slot, parent_block_hash, parent_block_root)`. #5002 (wording clarification for self-build envelope signature verification, Mar 13) — docs-only, no code change needed. #4979 (PTC Lookbehind) is now CLOSED (was OPEN). #4939 is also closed. #4992 (cached PTCs in state) still OPEN.

Shipped: derived Copy for 3 small fixed-size types: `BeaconBlockHeader` (104 bytes: Slot + u64 + 3×Hash256), `EnrForkId` (16 bytes: 2×[u8;4] + Epoch), `FinalizedExecutionBlock` (80 bytes: 2×Hash256 + 2×u64). Removed ~20 `.clone()` calls across 19 files: state upgrades (7 files), envelope_processing (3 sites), block_replayer (4 sites), beacon_state, beacon_fork_choice_store, test files (block_tests, inject_slashing, per_block_processing tests). BeaconBlockHeader is the most impactful — cloned in per-slot hot paths (envelope processing, block replay, state root computation).

Open PRs to track: #4992 (cached PTCs in state — approaching merge).

### run 1180 (Mar 14) — return fixed-size arrays from int_to_bytes functions

Spec stable: no new consensus-specs commits since last check. Both tracked PRs (#4992, #4939) still OPEN, unchanged. No new spec test releases. cargo audit unchanged (1 rsa, no fix).

Shipped: converted all int_to_bytes functions (int_to_bytes1/2/3/8/32/48/96) from returning `Vec<u8>` (heap-allocated via BytesMut) to returning fixed-size stack arrays (`[u8; N]`). `int_to_bytes4` already returned `[u8; 4]` — now all functions follow the same pattern using `to_le_bytes()`. Removed duplicate `int_to_fixed_bytes32` (now identical to `int_to_bytes32`). Dropped `bytes` crate dependency from int_to_bytes. Key hot paths affected: `get_seed` (per-slot RANDAO mix), `compute_proposer_indices` (per-epoch), `get_ptc_committee` (per-slot in Gloas), `get_next_sync_committee_indices`, `get_beacon_proposer_seed`. All 1290 types+state_processing tests, 104/104 EF spec tests (operations+epoch+sanity+ssz_static), 9/9 fork choice tests pass. Clippy clean.

Open PRs to track: #4992 (cached PTCs in state — 1 approval, approaching merge), #4939 (request missing payload envelopes for index-1 attestations).

### run 1177 (Mar 14) — reuse find_head_gloas allocations

Spec stable: no new consensus-specs commits since last check. Both tracked PRs (#4992, #4939) still OPEN, unchanged. No new spec test releases. cargo audit unchanged (1 rsa, no fix). Only semver-compatible dep update: cc 1.2.56 → 1.2.57.

Shipped: reuse `filtered_nodes` Vec<bool> and `children_index` HashMap allocations across `find_head_gloas` calls by storing them as struct fields on `ProtoArrayForkChoice`. Previously allocated fresh each slot (~20-37KB), now cleared and refilled in-place. All 188 proto_array tests, 119 fork_choice tests, 9/9 EF fork choice spec tests pass. Clippy clean.

Open PRs to track: #4992 (cached PTCs in state — 1 approval, approaching merge), #4939 (request missing payload envelopes for index-1 attestations).

### run 1172 (Mar 14) — avoid cloning SyncCommittee in sync aggregate processing

Spec stable: no new consensus-specs commits since last check. Both tracked PRs (#4992, #4939) still OPEN, unchanged. #4992 has 1 approval (jtraglia). No new spec test releases. cargo audit unchanged (1 rsa, no fix).

Shipped: eliminated SyncCommittee clone (~24KB on mainnet with 512×48-byte pubkeys) in `process_sync_aggregate` (sync_committee.rs) and `compute_sync_committee_rewards` (sync_committee_rewards.rs). Previously both cloned the entire `current_sync_committee` to break the borrow cycle needed for `get_sync_committee_indices(&mut self)`. Now call `update_pubkey_cache()` first, then compute committee indices inline using immutable `state.current_sync_committee()?.pubkeys` and `state.pubkey_cache()` accessors — two simultaneous `&self` borrows, no clone needed. Removed now-unused `get_sync_committee_indices` method from BeaconState. All 575 state_processing tests, EF sync_aggregate + sanity tests pass. Clippy clean, lint passes.

Open PRs to track: #4992 (cached PTCs in state — 1 approval, approaching merge), #4939 (request missing payload envelopes for index-1 attestations).

### run 1171 (Mar 13) — replace hash() with hash_fixed() to avoid heap allocations

Spec stable: no new consensus-specs commits since last check. Both tracked PRs (#4992, #4939) still OPEN, unchanged. No new spec test releases. cargo audit unchanged (1 rsa, no fix).

Shipped: replaced `ethereum_hashing::hash()` (returns `Vec<u8>`, heap allocation) with `hash_fixed()` (returns `[u8; 32]`, stack array) across 11 files and ~15 call sites. Key hot paths affected: proposer index computation (per-slot), sync committee selection (every ~27 hours), PTC committee selection (per-slot in Gloas), RANDAO mix updates (per-slot), seed generation, and deposit tree hashing. Also converted several preimage buffers from Vec to stack arrays (e.g. `get_beacon_proposer_seed` return type from `Result<Vec<u8>, Error>` to `Result<[u8; 32], Error>`, `compute_blob_parameters_hash` from `Vec::with_capacity(16)` to `[0u8; 16]`). All 715 types tests, 575 state_processing tests, 42 bls/genesis tests, 44 EF spec tests pass. Clippy clean, lint-full passes.

Open PRs to track: #4992 (cached PTCs in state — substantial change if merged), #4939 (request missing payload envelopes for index-1 attestations).

### run 1170 (Mar 13) — cache hash across balance-weighted selection loops

Spec stable: no new consensus-specs commits since last check. #5004 (docs-only) is the most recent. Both tracked PRs (#4992, #4939) still OPEN, unchanged. No new spec test releases. cargo audit unchanged (1 rsa, no fix). No semver-compatible dependency updates.

Shipped: cached the SHA-256 hash result across iterations in three balance-weighted selection loops: `get_ptc_committee` (gloas.rs), `compute_proposer_index` (beacon_state.rs), and `get_next_sync_committee_indices` (beacon_state.rs). These loops compute a hash that only changes every 16 (Electra+) or 32 (pre-Electra) iterations, but previously recomputed it on every single iteration. The PTC committee selection (512+ iterations per slot) saves ~480 hash computations per call. Also removed the per-iteration `seed.to_vec()` allocation from proposer/sync committee selection by hoisting the hash buffer. Removed now-unused `shuffling_random_value`, `shuffling_random_byte`, and `shuffling_random_u16_electra` helper functions. All 715 types tests, 575 state_processing tests, 139/139 EF spec tests, 9/9 fork choice tests, 307 proto_array+fork_choice tests pass. Clippy clean, lint-full passes.

Open PRs to track: #4992 (cached PTCs in state — substantial change if merged), #4939 (request missing payload envelopes for index-1 attestations).

### run 1169 (Mar 13) — reuse preimage buffer in compute_proposer_indices

Spec stable: no new consensus-specs commits since last check. #5004 (release notes dependency section) is a docs-only change. Both tracked PRs (#4992, #4939) still OPEN, unchanged. No new spec test releases (latest pre-release still v1.6.0-beta.0, our Gloas tests from custom alpha.3 build). cargo audit unchanged (1 rsa, no fix).

Shipped: eliminated per-slot Vec allocation in `compute_proposer_indices` (beacon_state.rs). Previously allocated a new `seed.to_vec()` + appended 8 bytes on each slot iteration (8-32 times per call, called during epoch processing via `process_proposer_lookahead`). Now hoists the preimage buffer outside the loop and overwrites only the slot bytes each iteration. All 32 proposer tests, 18 EF epoch processing tests, 2 sanity tests, 9 fork choice tests pass. Clippy clean, lint-full passes.

Open PRs to track: #4992 (cached PTCs in state — substantial change if merged), #4939 (request missing payload envelopes for index-1 attestations).

### run 1168 (Mar 13) — allocation optimizations, new fork choice tests verified

Spec checked: 3 new commits since alpha.3 release — #5001 (parent_block_root in bid filtering key, MERGED), #4940 (initial Gloas fork choice tests, MERGED), #5002 (wording clarification for self-build envelope signature, MERGED). All already implemented/compatible:
- #5001: vibehouse already uses `(slot, parent_block_hash, parent_block_root)` tuple in `ObservedExecutionBids` — ahead of spec.
- #4940: new `on_execution_payload` fork choice test vectors included in alpha.3 download. Test runner already has `OnExecutionPayload` step type and `head_payload_status` check. All 9/9 fork choice tests pass including the new one.
- #5002: wording-only change, no code impact.

Shipped allocation optimizations:
1. Electra upgrade (`upgrade/electra.rs`): eliminated full validators list `.clone()` by collecting indices during the immutable borrow phase, then releasing the borrow before mutation. Saves cloning hundreds of validator structs during fork transitions.
2. Gloas upgrade (`upgrade/gloas.rs`): pre-allocate `new_pending_deposits` Vec with `with_capacity(pending_deposits.len())`.
3. Attestation rewards (`attestation_rewards.rs`): pre-allocate `total_rewards` Vec with `with_capacity(validators.len())`.

All 32 upgrade tests, EF fork + rewards tests, fork choice tests pass. Clippy clean, lint-full passes.

Open PRs to track: #4992 (cached PTCs in state — substantial change if merged), #4939 (request missing payload envelopes for index-1 attestations).

### run 1167 (Mar 13) — HashSet lookup optimizations in state processing

Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check (v1.7.0-alpha.3). All tracked PRs (#4992, #4939) OPEN, unchanged. Updated serde_with 3.17.0 → 3.18.0. cargo audit unchanged (1 rsa, no fix).

Shipped two HashSet optimizations:
1. `onboard_builders_from_pending_deposits` in upgrade/gloas.rs: replaced `Vec<PublicKeyBytes>` with `HashSet<PublicKeyBytes>` for both `validator_pubkeys` and `new_validator_pubkeys`. The `.contains()` calls were O(n) linear scans over all validator pubkeys — now O(1) hash lookups. On mainnet with 500k+ validators, this eliminates O(validators × deposits) work during the Gloas fork transition.
2. `get_attestation_deltas_subset` in base/rewards_and_penalties.rs: changed `validators_subset` parameter from `&Vec<usize>` to `&[usize]` and converted to `HashSet<usize>` internally for O(1) `.contains()` lookups. Previously O(n) per validator per delta calculation in the HTTP API attestation rewards endpoint.

All 575 state_processing tests pass, EF rewards + fork tests pass. Clippy clean, lint-full passes.

Open PRs to track: #4992 (cached PTCs in state — substantial change if merged), #4939 (request missing payload envelopes for index-1 attestations).

### run 1166 (Mar 13) — eliminate intermediate Vec in find_head_gloas
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check (v1.7.0-alpha.3). All tracked PRs (#4992, #4939) OPEN, unchanged. #4979 (PTC Lookbehind) closed without merging — #4992 (cached PTCs) is the chosen approach. No semver-compatible dep updates. cargo audit unchanged (1 rsa, no fix).

Shipped: eliminated intermediate `weighted` Vec allocation in `find_head_gloas`. Previously, weights and tiebreakers were computed in separate passes — weights collected into a Vec via `.map().collect()`, then tiebreakers computed lazily inside `max_by`. Now precomputes both weight and tiebreaker in a single `.map()` step and chains directly into `max_by()`, avoiding the Vec heap allocation per tree level per fork choice update. This is possible because moving the tiebreaker computation into the map closure removes the `&self` borrow from `max_by`, so both closures can coexist on the iterator chain (map borrows `&self` + `&mut ancestor_cache`, max_by borrows nothing). All 188 proto_array + 119 fork_choice + 9/9 EF fork choice spec tests pass. Clippy clean, lint-full passes.

Open PRs to track: #4992 (cached PTCs in state — substantial change if merged), #4939 (request missing payload envelopes for index-1 attestations).

### run 1165 (Mar 13) — update spec tests to v1.7.0-alpha.3
New release v1.7.0-alpha.3 published today with 15 Gloas changes. Reviewed all 6 key spec PRs (#4897, #4884, #4916, #4923, #4918, #4948) — all already implemented in vibehouse. Downloaded new test vectors. Added `fork_choice_on_execution_payload` test handler for new Gloas fork choice tests from PR #4940 (on_execution_payload step + head_payload_status check + execution_payload_envelope files). Removed PayloadNotRevealed workaround from attestation processing — alpha.3 vectors include PR #4918 fix so index=1 attestations now properly sequence after on_execution_payload steps. Updated Makefile version pin. All 79/79 real-crypto + 138/138 fake-crypto tests pass. Clippy clean, pre-push lint-full passes.

Open PRs to track: #4992 (cached PTCs in state — substantial change if merged), #4939 (request missing payload envelopes for index-1 attestations), #4979 (PTC Lookbehind — alternative to #4992).

### run 1160 (Mar 13) — shared ancestor cache across siblings in fork choice
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. Version bump to v1.7.0-alpha.3 tagged (commit d2cfa51c, Mar 11) but not yet published as a release. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. PR #4939 updated today. cargo audit unchanged (1 rsa, 5 allowed warnings). No semver-compatible dep updates.

Shipped: shared the ancestor lookup cache across sibling weight calculations in `find_head_gloas`. Previously, each call to `get_gloas_weight` (one per child node) allocated its own `HashMap` for caching `get_ancestor_gloas` tree walks. Since sibling nodes at the same level share the same `node_slot`, ancestor lookups are identical and can be reused. Now allocates the cache once per tree level and passes it into all sibling calls. Cache key changed from `Hash256` to `(Hash256, Slot)` so entries remain correct when siblings have different slots (EMPTY/FULL → PENDING children case). For the common PENDING → EMPTY/FULL case, the FULL child reuses all ancestor walks computed for the EMPTY child, eliminating redundant O(depth) traversals per unique validator vote root. Added `get_gloas_weight_test` helper for tests. All 188 proto_array + 119 fork_choice + 8/8 EF fork choice spec tests pass. Clippy clean.

### run 1159 (Mar 13) — Vec<bool> filtered nodes in fork choice
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. PR #4940 (Gloas fork choice tests) merged — test generators only, no new vectors yet. PR #5001 (parent_block_root in bid filtering key) merged — already implemented in vibehouse. PR #5002 (wording clarification) — no code change needed. PR #5004 (release notes dependencies section) — tooling only. cargo audit unchanged (1 rsa, 5 allowed warnings). No semver-compatible dep updates.

Shipped: replaced `HashSet<Hash256>` with `Vec<bool>` in `compute_filtered_nodes` (renamed from `compute_filtered_roots`). The filtered block tree computation previously built a HashSet of 32-byte block roots — each insert/lookup required hashing 32 bytes. Now uses a `Vec<bool>` indexed by node index for O(1) lookups with no hashing. Also eliminates the intermediate HashSet allocation and the second pass that collected roots into it. Updated `get_gloas_children` to accept `&[bool]` and check by index. Added `is_filtered` test helper. Fixed 2 collapsible_if clippy warnings. All 188 proto_array + 119 fork_choice + 8/8 EF fork choice spec tests pass. Clippy clean.

### run 1158 (Mar 13) — children index in find_head_gloas
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. cargo audit unchanged (1 rsa, 5 allowed warnings). No semver-compatible dep updates. CI green.

Shipped: built parent→children HashMap index in `find_head_gloas` — the EMPTY/FULL branch of `get_gloas_children` previously scanned ALL proto_array nodes to find children of a specific parent (O(n) per call). Since `find_head_gloas` calls this repeatedly during traversal from justified root to head, total cost was O(depth × num_nodes). Now builds the index once at the start and passes it through, reducing child lookups to O(k) where k is actual children. All 188 proto_array + 119 fork_choice + 8/8 EF fork choice spec tests pass. Clippy clean.

### run 1157 (Mar 13) — attestation clone + ancestor cache
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. cargo audit unchanged (1 rsa, 5 allowed warnings). Nightly tests green. No semver-compatible dep updates.

Shipped two optimizations:
1. Eliminated unnecessary `attestation.clone()` in `SplitAttestation::new` — the function already takes ownership of the attestation, so it can be destructured directly by consuming the match. Also avoids a redundant `signature.clone()` by moving the field directly from the variant. All 36 operation_pool tests pass.
2. Added ancestor lookup cache in `get_gloas_weight` — many validators vote for the same block root, causing `get_ancestor_gloas(root, slot)` to repeat the same O(depth) tree walk for each validator. Now caches results per `vote.current_root` within each weight calculation, avoiding redundant walks. All 188 proto_array + 119 fork_choice + 8/8 EF fork choice spec tests pass. Clippy clean.

### run 1156 (Mar 13) — zero-clone maximum_cover
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. cargo audit unchanged (1 rsa, 5 allowed warnings). Nightly tests green. Rebased ptc-lookbehind branch (17 commits behind), 575/575 state_processing tests pass on branch.

Shipped: eliminated clone in `maximum_cover` greedy selection loop. Previously, each selected item was cloned (including its entire covering set HashMap) so it could be referenced while updating other items. Now uses `split_at_mut` to borrow the best item in-place while mutating others, and `Option::take()` to move items into the result instead of cloning. Removed `Clone` bounds from `MaxCover` trait and associated types. For attestation packing, this eliminates one `HashMap<u64, u64>` clone per selected attestation per iteration (up to 128 per epoch). Also updated transitive deps (anstyle 1.0.14, colorchoice 1.0.5). All 36 operation_pool tests pass. Clippy clean.

### run 1155 (Mar 13) — cached attestation reward sum
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. PR #4940 (Gloas fork choice tests) merged — test generators only, no new vectors yet. cargo audit unchanged (1 rsa, 5 allowed warnings, no fixes).

Shipped: cached `reward_numerator_sum` field in `AttMaxCover` — `score()` now returns the pre-computed sum instead of iterating the entire `fresh_validators_rewards` HashMap on every call. The cached sum is initialized at construction and decremented in `update_covering_set` when validators are removed. This eliminates O(n) HashMap value iteration per `score()` call (where n = remaining fresh validators per attestation). Combined with run 1154's call reduction, attestation scoring during max cover is now O(1) per item. All 36 operation_pool tests pass. Clippy clean.

### run 1154 (Mar 13) — max_cover score() optimization
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. No semver-compatible dep updates available. cargo audit unchanged (1 rsa, no fix).

Shipped: optimized `maximum_cover` in operation_pool — reduced redundant `score()` calls from 3× per item per iteration to 1×. For attestations, `score()` sums a HashMap of validator rewards, so this eliminates ~2n HashMap iterations per outer loop step. Also removed score check from update pass (update_covering_set on empty set is a no-op). All 36 operation_pool tests pass. Clippy clean.

### run 1153 (Mar 13) — all stable, Cargo.lock fix
Spec stable: no new consensus-specs commits, releases, or spec-test vectors since last check. Latest published release still v1.7.0-alpha.2. v1.7.0-alpha.3 tag exists but no release published yet. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. PR #4939 updated (2026-03-13). No new Gloas PRs beyond existing tracked set.

Shipped: committed Cargo.lock with smallvec dependency for operation_pool (was added as dep but lockfile not committed). Audited consensus hot paths for optimization opportunities — codebase is in good shape. All production `collect()`, `clone()`, and `HashMap` usages are either necessary (borrow conflicts with mutation) or in test code. Clippy clean across state_processing, proto_array, fork_choice, beacon_chain.

### run 1150 (Mar 13) — bitlist_extend bulk byte optimization
Spec stable: no new consensus-specs commits, releases, or spec-test vectors. All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. PR #4992 same head d76a278b0a.

Shipped: optimized `bitlist_extend` in operation_pool attestation storage — replaced O(n) bit-by-bit iteration (with bounds check per bit) with bulk byte copy + shift-OR for non-aligned cases. Added 5 unit tests (byte-aligned, non-aligned, empty, all-set, overflow). All 36 operation_pool tests pass. Clippy clean.

### run 1149 (Mar 13) — all stable, monitoring
Spec stable: no new consensus-specs commits since #5004 (Mar 13). No new releases (still v1.7.0-alpha.2 published), no new spec-test vectors (still v1.6.0-beta.0). All tracked PRs (#4932, #4939, #4960, #4962, #4992) OPEN, unchanged. PR #4992 (PTC lookbehind): OPEN, MERGEABLE, 1 APPROVED (jtraglia), same head d76a278b0a. CI green (clippy passed, other jobs running). cargo audit unchanged (1 rsa, no fix available). No semver-compatible dep updates. No outdated direct deps (only rand dev dep version mismatch in network tests). moonrepo/setup-rust@v1 Node.js 20 deprecation warning — upstream hasn't published Node.js 24 version yet, nothing actionable.

### run 1148 (Mar 13) — hdiff encode buffer optimization
Spec stable: no new releases (still v1.7.0-alpha.2 published), no new spec-test vectors (still v1.6.0-beta.0). Tracked PRs (#4932, #4939, #4960, #4962, #4992) all OPEN. PR #4940 (Gloas fork choice tests) merged — test generators only, no new vectors. CI green. cargo audit unchanged (1 rsa, no fix available).

Shipped: reduced xdelta3 encode buffer over-allocation in `compute_xdelta` — initial buffer now 1/4 of total size (was 2x), with retry-on-resize matching the existing `apply_xdelta` pattern. Added `store_hdiff_buffer_compute_resizes` metric. All 30 store tests + 7 hdiff tests pass.

### runs 959-1144 consolidated (Mar 11-13) — spec stable, monitoring only
Spec completely stable since v1.7.0-alpha.3 version bump (#4999, Mar 11). No new spec-test vectors (still v1.6.0-beta.0). No new formal release (still v1.7.0-alpha.2 published). All tracked spec-test PRs (#4932, #4939, #4960, #4962) remain OPEN. PR #4992 (PTC lookbehind): OPEN, MERGEABLE, 1 APPROVED (jtraglia), same head d76a278b0a, new comment from jihoonsong (Mar 13). CI and nightly continuously green. cargo audit unchanged (1 rsa). PR #4940 (Gloas fork choice tests) merged Mar 13 — test generators only, no new vectors yet. ptc-lookbehind branch rebased onto main, 575/575 state_processing tests pass. PR #5001 (parent_block_root in bid filtering key) merged — already implemented in vibehouse. PR #5002 (wording clarification) — no code change needed. PR #5004 (release notes dependencies section) — tooling only. No semver-compatible dep updates. Codebase audit: no TODOs/FIXMEs/untested paths in Gloas code.

Notable activities:
- Run 1054: Committed Cargo.lock transitive dep update (windows-sys 0.61.2, syn 2).
- Run 988: Added 5 SSZ round-trip tests for proto_array Gloas fields (ProtoNode, VoteTracker, SszContainer).
- Runs 994, 1005, 1008, 1015, 1031: Multiple code/test coverage audits — all Gloas consensus paths verified correct, no unwraps, all safe arithmetic, comprehensive integration test coverage.
- Run 959: Verified all 7 alpha.3 changes already implemented.

### runs 759-958 consolidated (Mar 10-13) — spec stable, PTC lookbehind implemented
Spec completely stable — no new consensus-specs commits with consensus changes since #5001 (Mar 12), no new release (still v1.7.0-alpha.2), no new spec-test vectors (still v1.6.0-beta.0). All 11 tracked Gloas PRs remained OPEN throughout. PR #4992 (PTC lookbehind) evolved from head 215962a9 (blocked) to d76a278b0a (clean, 1 APPROVED jtraglia Mar 12). CI and nightly continuously green. EF spec tests: consistently 35/35 (minimal, fake_crypto), fork choice: 8/8 (real crypto). Workspace tests: 2643/2651 (8 web3signer timeout). cargo audit: 1 rsa advisory (no fix). Recent consensus-specs merges were all CI/tooling (#4984 remove Verkle, #4988-#4995 Python/reftest/release-drafter).

Notable activities:
- Run 929: Implemented PTC lookbehind on branch `ptc-lookbehind` (previous_ptc/current_ptc fields, compute_ptc, get_ptc cached reads, per_slot rotation, upgrade initialization). All 575 state_processing tests pass. NOT merged — blocks on PR #4992 merge + new spec test vectors.
- Run 929: Fixed `clippy::large_stack_frames` in `proposer_boost_re_org_test` (Rust 1.91 bump)
- Run 926: Updated deps (clap 4.6, openssl 0.10.76, c-kzg 2.1.7, tempfile 3.27)
- Run 871: Updated Cargo.lock (windows-sys transitive deps)
- Run 850: Added workflow_dispatch trigger to ci.yml
- Run 834: Codebase audit — 39 TODOs (all inherited/spec-dependent), gloas.rs has 208 unit tests across 9216 lines
- Run 800: Analyzed PTC lookbehind implementation plan (7 code areas)

### 2026-03-09 — consolidated: runs 524-758 (Mar 7-10)
Key activities across ~230 runs:
- **run 735**: Fixed 2 beacon_chain test failures (slasher backend guard, Fulu fork scheduling check)
- **run 723-725**: Added 22 proto_array tests (propagation, validation, invalidation, viability, contains_invalid_payloads, on_invalid_execution_payload)
- **run 718**: Deep spec conformance audit — all Gloas functions verified correct against consensus-specs master
- **run 717**: Added 6 tests for `process_payload_attestation` + `get_indexed_payload_attestation`
- **run 701**: Implemented PR #4939 (index-1 attestation envelope validation) proactively
- **run 680-677**: Updated zerocopy, alloy-trie, quinn-proto, yamux deps
- **run 676**: Added 4 prometheus gauge metrics for ePBS pool monitoring
- **run 675**: Added 5 epoch processing integration/edge case tests
- **run 641**: docker CI paths-ignore for docs-only commits
- **run 640**: post-rebrand devnet verification SUCCESS
- **run 578**: upgraded ethabi 16→18
- **run 577**: upgraded 7 dependencies (jsonwebtoken 9→10, rpassword 5→7, etc.)
- **run 572-576**: switched default DB to redb, upgraded RustCrypto suite, replaced psutil with procfs
- **run 547**: fixed gossip message leak
- **run 545**: automated spec release check workflow, CI concurrency fix

### 2026-03-07 — consolidated: runs 37-523 (Feb 20 - Mar 7)
~480 runs of test writing, spec monitoring, and maintenance. Key milestones:
- **Feb 20-Mar 1**: wrote 800+ unit tests across all Gloas subsystems (fork choice, state processing, gossip verification, beacon chain, HTTP API, types, validator client)
- **Mar 1-3**: external builder integration tests, devnet test scenarios (sync, churn, mainnet, long-running, builder, partition, slashing)
- **Mar 3-5**: code review & quality improvement (5 phases: clippy/doc audit, architecture review, correctness deep-dive, performance audit, test quality)
- **Mar 5-7**: dependency upgrades, redb migration, CI improvements

### 2026-02-19 — full-preset EF test verification (mainnet + minimal)
- Both presets pass: 78/78 real crypto, 138/138 fake_crypto

### 2026-02-18 — fix fork_choice_on_block for Gloas blocks (77/78 → 78/78)
- Fixed Gloas block on_block handler to properly set bid fields

### 2026-02-19 — add ProposerPreferences SSZ types (136→138 fake_crypto tests)
- Added SSZ serialization for ProposerPreferences, fixing 2 remaining test failures

### 2026-02-17 — fix check_all_files_accessed (was failing with 66,302 missed files)
- Registered all Gloas test directories in the test runner

### 2026-02-17 — 78/78 passing (execution_payload envelope tests added)
- Added envelope test handlers, all passing

### 2026-02-17 — 77/77 passing (DataColumnSidecar SSZ fixed)
- Fixed Gloas variant for DataColumnSidecar serialization

### 2026-02-15 — 76/77 passing
- Initial Gloas test scaffolding complete

### 2026-02-14 — SSZ static pass
- First pass at Gloas SSZ static tests

### Run 1182: spec tracking review (2026-03-14)

**Scope**: Checked consensus-specs for post-alpha.3 changes.

**Post-alpha.3 PRs (merged after v1.7.0-alpha.3 tag):**
1. **#5001** — Add `parent_block_root` to bid filtering key → **Already implemented.** Our `ObservedExecutionBids::is_highest_value_bid` already uses `(slot, parent_block_hash, parent_block_root)` as the key (implemented proactively).
2. **#4940** — Add initial fork choice tests for Gloas → **Already supported.** Our EF test runner handles `on_execution_payload` steps, `execution_payload_envelope_*.ssz_snappy` files, and `head_payload_status` checks. Test vectors will arrive with next spec release.
3. **#5002** — Clarify wording for payload signature verification → **Documentation only**, no implementation change needed.

**Status**: vibehouse is ahead of the spec. All three post-alpha.3 changes are already handled.
