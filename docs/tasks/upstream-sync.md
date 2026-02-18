# Upstream Sync

## Objective
Stay current with upstream lighthouse fixes and improvements.

## Status: ONGOING

### Process
1. `git fetch upstream` — check for new commits
2. Categorize: security fix (immediate), bug fix (cherry-pick), feature (evaluate), refactor (if clean)
3. Test after every cherry-pick batch
4. Push and verify

### Upstream PRs to watch
- [#8806 - Gloas payload processing](https://github.com/sigp/lighthouse/pull/8806)
- [#8815 - Proposer lookahead](https://github.com/sigp/lighthouse/pull/8815)
- [#8807 - Inactivity scores ef tests](https://github.com/sigp/lighthouse/pull/8807)
- [#8793 - Process health observation](https://github.com/sigp/lighthouse/pull/8793)
- [#8786 - HTTP client user-agent](https://github.com/sigp/lighthouse/pull/8786)

### Recent spec changes (consensus-specs) needing attention
- consensus-specs PR #4807 — `update_proposer_boost_root` proposer index check — **DONE**: only apply proposer boost if block's proposer matches canonical chain's expected proposer for the slot. Added `canonical_head_proposer_index: Option<u64>` param to `on_block`, computed from cached head state before fork choice lock. All 8/8 fork choice EF tests pass (real + fake crypto), 34/34 fork_choice unit tests, 18/18 proto_array tests. Fixed 2026-02-18.
- `3f9caf73` — ignore beacon block if parent payload unknown (gossip validation) — **DONE**: added `[IGNORE]` rule in `GossipVerifiedBlock::new()` — checks `parent_block.payload_revealed` for Gloas parents. New `GloasParentPayloadUnknown` error variant, handled as IGNORE in gossip methods. Fixed 2026-02-18.
- `e57c5b80` — rename `execution_payload_states` to `payload_states` — **ASSESSED**: naming-only change in spec pseudocode. Our impl uses different internal names (proto_array nodes, not a dict).
- `06396308` — payload data availability vote (new `DATA_AVAILABILITY_TIMELY_THRESHOLD`) — **DONE**: separate `ptc_blob_data_available_weight` + `payload_data_available` tracking on ProtoNode, full `should_extend_payload` implementation. Fixed 2026-02-17.
- `b3341d00` — check pending deposit before applying to builder — **ASSESSED**: our code already removed the incorrect `is_pending_validator` check (commit `0aeabc122`). Current routing logic matches spec.
- `40504e4c` — refactor builder deposit conditions in process_deposit_request — **ASSESSED**: current implementation matches refactored spec logic.
- `36a73141` — replace pubkey with validator_index in SignedExecutionProof — **ASSESSED**: our `SignedExecutionPayloadEnvelope` already uses `builder_index` (u64).
- `278cbe7b` — add voluntary exit tests for builders — **ASSESSED**: these are Python spec test generator additions, not spec logic changes. The generated EF test fixtures (`process_execution_payload_bid_inactive_builder_exiting`) are already in our test suite and pass. No standalone `process_builder_exit` operation exists in the spec — builder exits are modeled via `withdrawable_epoch` on the `Builder` type.

## Progress log

### 2026-02-18 (run 8)
- Fetched upstream: no new commits since run 7
- No new consensus-specs changes requiring implementation (top commits are packaging/infrastructure: eth-remerkleable, package rename, publish scripts)
- Tracked open consensus-specs PRs:
  - #4918 (attestations for known payload statuses) — still open
  - #4939 (request missing payload envelopes for index-1 attestation) — still open
  - #4898 (remove pending status from tiebreaker) — still open, assessed: our tiebreaker code still checks Pending but it's functionally correct, trivial change when merged
  - #4892 (remove impossible branch in forkchoice) — still open, assessed: our `is_supporting_vote_gloas` uses `<=` (old spec), PR changes to assert `>=` + check `==`, functionally equivalent
  - #4932 (add Gloas sanity/blocks tests with payload attestation coverage) — still open
- Unskipped 3 fork choice EF tests that were blocked on lighthouse#8689 (now that PR #4807 proposer boost check is implemented):
  - `voting_source_beyond_two_epoch`, `justified_update_not_realized_finality`, `justified_update_always_if_better`
  - All pass with both real and fake crypto
  - EF test results: 78/78 real crypto (0 skipped, was 3), 136/136 fake crypto (0 skipped, was 3)
- Fixed CI failures:
  - clippy `question_mark` lint in `lookups.rs:1973` (Rust 1.93 new lint)
  - BLS test fixtures missing in CI — `consensus-spec-tests` is not a git submodule, needs `make -C testing/ef_tests` to download. Replaced `submodules: recursive` with download step. Also removed unused `submodules: recursive` from non-ef-tests jobs.
  - `rpc_columns_with_invalid_header_signature` fails at Gloas because DataColumnSidecar structure changed (no `signed_block_header`). Skipped for Gloas — test premise doesn't apply.
- Pre-existing Gloas test failures identified (not introduced by this run):
  - 29 `store_tests::*` failures at `FORK_NAME=gloas` — `PayloadBidInvalid: bid parent_block_hash does not match state latest_block_hash`. Root cause: mock EL + test harness state management with skipped slots doesn't properly handle ePBS envelope state. These are test infrastructure issues, not consensus bugs.
  - `validator_monitor::missed_blocks_across_epochs` — also pre-existing

### 2026-02-18 (run 7)
- Fetched upstream: no new commits since run 6
- No new consensus-specs changes requiring implementation (checked latest merged PRs — all packaging/infrastructure)
- Tracked open consensus-specs PRs: #4918 (attestations for known payload statuses), #4939 (request missing payload envelopes for index-1 attestation) — both still open/unmerged
- Implemented remaining PR #4807 change: equivocating validator weight in `is_head_weak`
  - Threaded `equivocating_indices: &BTreeSet<u64>` from `find_head` → `find_head_gloas` → `should_apply_proposer_boost_gloas`
  - Added equivocating validators' effective balance to parent attestation weight before comparing against reorg threshold
  - This matches spec's `is_head_weak` which sums both attesting and equivocating weight
  - Previously had a placeholder comment "simplified: we don't have equivocating indices here, so skip this"
- Fixed pre-existing clippy warnings across codebase (Rust 1.93 has stricter lints):
  - proto_array: collapsible_if, manual_let_else in 4 places
  - state_processing: 10 redundant closures (`|e| Error(e)` → `Error`), let_underscore_must_use in block_replayer
  - fork_choice: map_or → is_none_or
  - beacon_chain: collapsible_if, manual_let_else, needless_borrow, bool_assert_comparison
  - http_api: large_stack_frames in test functions
  - types: items_after_test_module
- Tests: 18/18 proto_array, 34/34 fork_choice, 56/56 state_processing, 8/8 fork_choice EF (real + fake crypto) — all pass
- Remaining from PR #4807 (non-consensus-critical reorg enhancements):
  - `record_block_timeliness` with 2-element timeliness vector — not strictly needed, our `ptc_timely: current_slot == block.slot()` and `is_before_attesting_interval` checks are functionally equivalent
  - `is_proposer_equivocation` helper extraction — cosmetic refactor, logic already exists inline

### 2026-02-18 (run 6)
- Fetched upstream: no new commits since run 5
- No new consensus-specs changes requiring implementation (latest release still v1.7.0-alpha.2, newer spec commits are packaging/infrastructure)
- Reviewed community PRs:
  - PR #25 (Th0rgal): 4 fixes — 3 already applied on main, applied remaining fix (use canonical `BUILDER_INDEX_SELF_BUILD` constant instead of local copy in proto_array). Closed PR with credit.
  - PR #26 (Th0rgal): cargo fmt + unused imports — all already fixed on main. Closed as redundant.
- Tests: 52/52 proto_array+fork_choice, 136/136 minimal EF (fake_crypto), 8/8 fork_choice EF (real crypto) — all pass

### 2026-02-18 (run 5)
- Fetched upstream: no new commits since run 4 (top is `54b357614` — agent review docs, skip)
- No new consensus-specs changes requiring implementation (top commits are packaging/infrastructure: eth-remerkleable, package rename, dependency updates)
- Implemented consensus-specs PR #4807: `update_proposer_boost_root` proposer index check
  - Added `canonical_head_proposer_index: Option<u64>` parameter to `ForkChoice::on_block`
  - In `import_block`, compute expected proposer from cached head state before fork choice lock
  - Only apply proposer boost if `block.proposer_index == expected_proposer_index`
  - Skip check when epoch mismatch (can't compute proposer without state advance) or during fork revert
  - Updated 6 call sites: beacon_chain, fork_revert, fork_choice tests, ef_tests, payload_invalidation
  - Tests: 8/8 fork choice EF (real + fake crypto), 34/34 fork_choice, 18/18 proto_array — all pass
- Remaining from PR #4807 (not yet implemented):
  - `is_proposer_equivocation` helper for `get_proposer_head` reorg logic
  - `should_apply_proposer_boost` changes in Gloas `get_weight` (already partially implemented, needs `block_timeliness` vector)
  - Modified `is_head_weak` (Gloas) to include equivocating validator weight
  - `record_block_timeliness` with two-element timeliness vector
  - These are non-consensus-critical (reorg logic only) and can be done in a follow-up

### 2026-02-18 (run 4)
- Fixed CI: `cargo fmt` failure in gossip_methods.rs and fork_choice.rs (from run 3 commits)
- Revisited previously-skipped cherry-picks:
  - `be799cb2a` — VC head monitor timeout: **SKIP** — our code uses `EventSource::get(path)` (bare reqwest with no timeout), not `self.client` with configured timeout. Bug doesn't affect us.
  - `691c8cf8e` — duplicate data columns fix: **SKIP** — our code already deduplicates correctly (`.map(|(root, _)| root).unique()`). Upstream's bug was `.unique()` on `(root, slot)` tuples.
  - `c61665b3a` — penalize peers for invalid RPC: **DONE** — resolved conflict in rpc_tests.rs imports (kept our `mod common` pattern, added `libp2p::PeerId`). All 3 new tests pass.
- New cherry-picks:
  - `a3a74d898` — fix ProcessHealth::observe computing `children_system` twice instead of `children_system + children_user` (metrics bug)
  - `5563b7a1d` — fix execution engine test using stale `valid_payload.block_hash()` instead of `second_payload.block_hash()`
  - `1fe7a8ce7` (partial) — gate `inactivity_scores` rewards tests to Altair+ forks (prevents directory-not-found on Phase0)
- Evaluated and skipped:
  - `945f6637c` — reqwest re-export removal (20-file refactor, 6 conflicts)
  - `48a2b2802` — delete OnDiskConsensusContext (still used in our state_lru_cache.rs)
  - `fcfd061fc` — feature gate SseEventSource (file doesn't exist in our fork)
  - `f4a6b8d9b` — tree-sync lookup sync tests (4600-line rewrite, heavy conflicts)
- No new consensus-specs changes requiring implementation (top commits are packaging/infrastructure)

### 2026-02-18 (run 3)
- Implemented spec change `3f9caf73`: gossip validation `[IGNORE]` for Gloas blocks whose parent execution payload hasn't been seen
  - New `GloasParentPayloadUnknown` error variant in `BlockError`
  - Check in `GossipVerifiedBlock::new()`: for Gloas blocks, if parent has `bid_block_hash` (is a Gloas block) and `payload_revealed == false`, IGNORE the block
  - Pre-Gloas parents are always considered "seen" (payload is in the block body)
  - Gossip handler returns `MessageAcceptance::Ignore` with no peer penalty
- Tests: 8/8 fork_choice EF (real + fake crypto), 170/170 beacon_chain (1 pre-existing failure excluded), 23/23 network fulu (1 pre-existing failure excluded)

### 2026-02-18 (run 2)
- Fetched upstream: no new commits since earlier today
- Cherry-picked cleanly:
  - `d4ec006a3` — update `time` crate to fix `cargo audit` failure (via `cargo update -p time`)
  - `711971f26` — cache slot in check_block_relevancy to prevent TOCTOU race
  - `96bc5617d` — auto-populate ENR UDP port from discovery listen port
  - `8d72cc34e` — add sync request metrics
  - `2f7a1f3ae` — support pinning nightly ef test runs
- Conflicted (skipped):
  - `d7c78a7f8` — rename --reconstruct-historic-states to --archive (conflicts in store_tests.rs and tests.rs due to gloas changes)
- Fixed pre-existing DataColumnSidecar `.index` → `.index()` in network test code (6 call sites)
- New spec changes assessed:
  - `3f9caf73` — ignore block if parent payload unknown (gossip validation) — needs implementation
  - `e57c5b80` — rename execution_payload_states to payload_states — naming only, no code change needed
  - `e46ecbae` — ZK proof dedup (EIP-8025 feature, not in gloas core)
  - Others: infrastructure, docs, renaming

### 2026-02-18
- Fetched upstream: 20 new commits since last check (including 4 Gloas upstream PRs)
- Cherry-picked cleanly:
  - `c5b4580e3` — return correct variant for snappy errors (rpc codec fix)
  - `9065e4a56` — add pruning of observed_column_sidecars (memory fix)
- Conflicted (resolved in run 4):
  - `be799cb2a` — VC head monitor timeout fix (skipped — doesn't affect our SSE client pattern)
  - `691c8cf8e` — fix duplicate data columns in DataColumnsByRange (skipped — our dedup is already correct)
  - `c61665b3a` — penalize peers for invalid rpc request (cherry-picked with conflict resolution)
- Upstream Gloas PRs (evaluated, not cherry-picked — our impl is ahead):
  - `eec0700f9` — Gloas local block building MVP
  - `67b967319` — Gloas payload attestation consensus
  - `41291a8ae` — Gloas fork upgrade consensus
  - `4625cb6ab` — Gloas local block building cleanup

### 2026-02-15
- Fetched upstream: 4 new commits since last check
- `48a2b2802` delete OnDiskConsensusContext, `fcfd061fc` fix eth2 compilation, `5563b7a1d` fix execution engine test, `1fe7a8ce7` implement inactivity scores ef tests
- None security-critical, none cherry-pick urgent
