# Session: Planning Documentation for Remaining Phase 4 Work

**Date**: 2026-02-14 16:21-16:50 GMT+1
**Agent**: ethvibes (Vibehouse Worker)
**Duration**: ~30 minutes
**Branch**: phase4-validation-wiring

## Context

- **Phase 4 P2P Networking**: 4/7 complete entering this session
- **Blocker**: Rust toolchain issue (bitvec 0.17.4 incompatible with newer Rust)
- **Goal**: Document remaining work so it's ready to execute when toolchain is fixed

## Work Completed

### 1. Beacon Processor Integration Plan

**File**: `docs/workstreams/gloas-beacon-processor-integration.md`

**Scope**: Wire gloas ePBS gossip messages into the beacon processor pipeline

**Key Sections**:
- **Work Variants**: Add 3 new variants to `Work<E>` enum
  - `GossipExecutionBid(BlockingFn)`
  - `GossipExecutionPayload(BlockingFn)`
  - `GossipPayloadAttestation(BlockingFn)`

- **Process Methods**: Detailed specs for 3 handlers in `gossip_methods.rs`:
  ```rust
  process_gossip_execution_bid()
  process_gossip_execution_payload()
  process_gossip_payload_attestation()
  ```

- **Integration Points**: 
  - Fork choice connection (`chain.on_execution_bid()`, `chain.on_payload_attestation()`)
  - Equivocation handling (reject + mark + peer penalty)
  - Error categorization (invalid sig, unknown parent, future slot)

- **Testing Requirements**: Unit tests + integration tests defined

- **Estimated Effort**: 6 hours focused implementation

**Dependencies**:
- ✅ Gossip validation types (already done)
- ✅ Fork choice handlers (already done)
- ⏳ Work variants (this plan)
- ⏳ Router wiring (this plan)
- ⏳ Metrics (this plan)

### 2. Peer Scoring Configuration Plan

**File**: `docs/workstreams/gloas-peer-scoring.md`

**Scope**: Configure gossipsub peer scoring for gloas ePBS topics

**Key Decisions**:

| Topic | Weight | Expected Rate | Rationale |
|-------|--------|---------------|-----------|
| ExecutionBid | 0.5 | 1/slot | Critical consensus message, same as BeaconBlock |
| ExecutionPayload | 0.5 | 1/slot | Withholding is slashable, high importance |
| PayloadAttestation | 0.4 | ~307/slot | Many messages, slightly lower weight |

**Penalty Strategy**:
- Invalid signature → graylist threshold (-16000)
- Equivocation → permanent mark + graylist
- Future slot → small penalty (-100), retry later
- Unknown parent → no penalty, reprocess queue

**Implementation Location**:
- File: `beacon_node/lighthouse_network/src/service/gossipsub_scoring_parameters.rs`
- Add weight constants at top
- Insert topic params in `get_peer_score_params()`
- Update `max_positive_score` calculation
- Guard with `gloas_enabled()` check

**Open Questions**:
1. Should payload attestations be aggregated before gossip?
2. Do builders need per-epoch rate limiting?
3. Is 307 attestations/slot sustainable bandwidth?

**Estimated Effort**: 2 hours (30 min params + 15 min integration + 1h testing)

## Session Commits

1. `b4bceea5f` - docs: complete beacon processor integration plan for gloas ePBS messages
2. `be8ac618b` - docs: peer scoring configuration plan for gloas ePBS topics
3. `f048f13a6` - plan: update phase 4 status (4/7 complete, 2 implementation plans ready)

## Updated Status

### Phase 4 P2P Networking: 4/7 Complete

**Complete** ✅:
1. Gossip topics (`execution_bid`, `execution_payload`, `payload_attestation`)
2. Validation infrastructure (`gloas_verification.rs`, error types, verified wrappers)
3. Equivocation detection (`observed_execution_bids.rs`, `observed_payload_attestations.rs`)
4. Validation wiring (builder registry, signature verification) - PR #18

**Planned** 📋 (8 hours total):
5. Beacon processor integration (6h)
6. Peer scoring configuration (2h)

**Remaining** ⏳:
7. Tests (integration + unit)

## Next Session Goals

1. **Fix toolchain issue**: Resolve bitvec 0.17.4 compatibility
2. **Execute beacon processor plan**: Implement Work variants + process methods
3. **Execute peer scoring plan**: Add topic params + weight constants
4. **Write tests**: Full gossip → fork choice integration tests

## Key Insights

1. **Documentation-driven development works**: Having comprehensive plans ready means we can execute quickly when blockers are removed
2. **Planning is valuable even when blocked**: 8 hours of implementation is now spec'd and ready to go
3. **Open questions are features, not bugs**: Documenting unknowns (aggregation, rate limits, bandwidth) helps future decision-making

## Metrics

- **Planning time**: 30 minutes
- **Work planned**: 8 hours
- **Documents created**: 2 (7.4KB + 6.8KB = 14.2KB)
- **Commits**: 3
- **Progress**: Phase 4 advanced from 4/7 to 4/7+2 planned (readiness increased significantly)

---

**Status**: Planning session successful. Phase 4 ready for rapid execution when toolchain is available. 🎵
