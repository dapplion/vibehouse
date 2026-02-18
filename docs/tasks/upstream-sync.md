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
- `06396308` — payload data availability vote (new `DATA_AVAILABILITY_TIMELY_THRESHOLD`) — **DONE**: separate `ptc_blob_data_available_weight` + `payload_data_available` tracking on ProtoNode, full `should_extend_payload` implementation. Fixed 2026-02-17.
- `b3341d00` — check pending deposit before applying to builder — **ASSESSED**: our code already removed the incorrect `is_pending_validator` check (commit `0aeabc122`). Current routing logic matches spec.
- `40504e4c` — refactor builder deposit conditions in process_deposit_request — **ASSESSED**: current implementation matches refactored spec logic.
- `36a73141` — replace pubkey with validator_index in SignedExecutionProof — **ASSESSED**: our `SignedExecutionPayloadEnvelope` already uses `builder_index` (u64).
- `278cbe7b` — add voluntary exit tests for builders — NOT YET ASSESSED

## Progress log

### 2026-02-18
- Fetched upstream: 20 new commits since last check (including 4 Gloas upstream PRs)
- Cherry-picked cleanly:
  - `c5b4580e3` — return correct variant for snappy errors (rpc codec fix)
  - `9065e4a56` — add pruning of observed_column_sidecars (memory fix)
- Conflicted (skipped, may revisit):
  - `be799cb2a` — VC head monitor timeout fix (our SSE client init differs)
  - `691c8cf8e` — fix duplicate data columns in DataColumnsByRange (rpc_methods conflict)
  - `c61665b3a` — penalize peers for invalid rpc request (test conflict)
- Upstream Gloas PRs (evaluated, not cherry-picked — our impl is ahead):
  - `eec0700f9` — Gloas local block building MVP
  - `67b967319` — Gloas payload attestation consensus
  - `41291a8ae` — Gloas fork upgrade consensus
  - `4625cb6ab` — Gloas local block building cleanup

### 2026-02-15
- Fetched upstream: 4 new commits since last check
- `48a2b2802` delete OnDiskConsensusContext, `fcfd061fc` fix eth2 compilation, `5563b7a1d` fix execution engine test, `1fe7a8ce7` implement inactivity scores ef tests
- None security-critical, none cherry-pick urgent
