# Upstream Sync Tracking

> tracking lighthouse upstream changes relevant to vibehouse

## Latest Sync: 2026-02-14

### Security Fixes (Applied)

- ✅ **c25a97592** - Bump bytes to 1.11.1 to fix RUSTSEC-2026-0007 (#8743)
  - **Status**: Cherry-picked manually (Cargo.toml change only)
  - **Commit**: 2e36cf662

### Gloas Implementation Progress (Upstream)

Major gloas work landed on upstream/unstable in the last 24h:

#### ✅ 68ad9758a - Gloas attestation verification (#8705)
- **Merged**: Feb 13, 2026
- **Changes**:
  - Implements gloas attestation verification per p2p spec
  - Adds 259 lines of new tests in beacon_chain/tests/attestation_verification.rs
  - Updates attestation_verification.rs with gloas-specific logic (+68 lines)
  - Updates gossip_methods.rs to handle gloas attestations
- **Files**: 3 files, +336/-10 lines
- **Status**: Not yet evaluated for cherry-pick (needs review)

#### ✅ 26db01642 - Gloas consensus: epoch processing, block signature verification, more tests (#8808)
- **Merged**: Feb 13, 2026
- **Changes**:
  - Implements `process_builder_pending_payments` in epoch processing
  - Updates `include_all_signatures_except_proposal` for gloas bid signature
  - Enables gloas for ALL remaining EF tests (except finality tests)
  - Adds epoch processing tests
  - Updates per_slot_processing with gloas logic
- **Files**: 9 files, +192/-34 lines
- **Key insight**: EF tests are mostly passing for gloas now!
- **Status**: High priority to review - this is core consensus logic

#### ✅ b8072c5b7 - Gloas payload bid consensus (#8801)
- **Merged**: Feb 12, 2026
- **Changes**:
  - Consensus changes for execution payload bids
  - Adds bid processing to per_block_processing (+170 lines)
  - New signature sets for bid verification
  - New error types for bid validation
  - EF tests for bids enabled
- **Files**: 10 files, +358/-18 lines
- **Status**: Critical - this is ePBS core bid processing

### Other Upstream Changes (Unstable)

- f4a6b8d9b - Tree-sync friendly lookup sync tests (#8592) - testing improvement
- c59e4a0ce - Disable `legacy-arith` by default in `consensus/types` (#8695) - type system cleanup
- 036ba1f22 - Add `network` feature to `eth2` (#8558) - feature flag work
- 96bc5617d - fix: auto-populate ENR UDP port from discovery listen port (#8804) - p2p fix
- 711971f26 - fix: cache slot in check_block_relevancy to prevent TOCTOU (#8776) - bug fix
- d7c78a7f8 - rename --reconstruct-historic-states to --archive (#8795) - CLI change
- 8d72cc34e - Add sync request metrics (#7790) - metrics
- 889946c04 - Remove pending requests from ready_requests (#6625) - p2p cleanup
- e1d3dcc8d - Penalize peers that send an invalid rpc request (#6986) - peer scoring
- 56eb81a5e - Implement weak subjectivity safety checks (#7347) - safety feature
- a1176e77b - Add insecure-dep test task to Makefile and CI (#8464) - CI tooling
- 8948159a4 - Add AI assistant documentation and commands (#8785) - (already cherry-picked)

### Upstream PRs to Watch

Still tracking:
- #8806 - Gloas payload processing (may have been superseded by merged PRs)
- #8815 - Proposer lookahead (still open)

### Cherry-pick Strategy

**Immediate priorities**:
1. ~~Security fix (c25a97592)~~ ✅ Done
2. Review gloas consensus PRs before cherry-picking - these are large changes and we need to understand them first
3. Consider whether to cherry-pick incrementally or wait for more stability

**Decision needed**: Should we cherry-pick gloas work now, or continue with spec-first documentation approach?

**Current thinking**: 
- Document first: read the merged PRs, understand the implementation choices
- Then decide: cherry-pick vs implement independently based on spec
- Advantage of cherry-pick: less work, proven to pass EF tests
- Advantage of independent: learn by doing, may catch issues
- Hybrid approach: cherry-pick the types and basic structure, verify against spec

### Non-Gloas Cherry-Pick Candidates

- 711971f26 - TOCTOU fix in block relevancy check (bug fix, low risk)
- 96bc5617d - ENR UDP port auto-populate (p2p improvement)
- d7c78a7f8 - CLI rename for historic states (breaking change, consider carefully)

### Actions for Next Cycle

1. Read the merged gloas PRs in detail (b8072c5b7, 26db01642, 68ad9758a)
2. Compare against consensus-specs to verify correctness
3. Decide: cherry-pick vs implement from spec
4. Document the decision in this file
5. If cherry-picking, do it in order: types -> bid processing -> epoch processing -> attestation verification
