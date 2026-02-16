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
- `06396308` — payload data availability vote (new `DATA_AVAILABILITY_TIMELY_THRESHOLD`)
- `b3341d00` — check pending deposit before applying to builder
- `40504e4c` — refactor builder deposit conditions in process_deposit_request
- `36a73141` — replace pubkey with validator_index in SignedExecutionProof
- `278cbe7b` — add voluntary exit tests for builders

## Progress log

### 2026-02-15
- Fetched upstream: 4 new commits since last check
- `48a2b2802` delete OnDiskConsensusContext, `fcfd061fc` fix eth2 compilation, `5563b7a1d` fix execution engine test, `1fe7a8ce7` implement inactivity scores ef tests
- None security-critical, none cherry-pick urgent
