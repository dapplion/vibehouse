# vibehouse plan

> the vibemap. not a roadmap. roadmaps have deadlines. vibemaps have directions.

## identity

vibehouse is an independent Ethereum consensus client. It shares historical code ancestry with Lighthouse v8.0.1 but that relationship is over. vibehouse is its own project now.

**⚠️ FULLY INDEPENDENT.** NEVER look at, reference, cherry-pick, merge, or pull any code from sigp/lighthouse. Do not read upstream PRs or diffs for guidance. Do not add upstream remotes. The only references are the Ethereum consensus-specs, EIPs, and vibehouse's own codebase. We write our own code from spec. Exception: if dapplion explicitly says to adopt something specific from upstream, do it.

---

## priorities

### 1. Kurtosis 4-node devnet — DONE

**Result**: 4 vibehouse CL + geth EL nodes, finalized_epoch=8 (slot 80, epoch 10). No stalls.

- Config: minimal preset, gloas fork at epoch 1, 4 participants (all vibehouse), spamoor + dora
- Script polls beacon API directly; package pinned to `ethereum-package@6.0.0`
- See `docs/tasks/kurtosis-devnet.md` for full progress log

**Commands:**
```bash
scripts/kurtosis-run.sh           # Full lifecycle (build + start + poll + teardown)
scripts/kurtosis-run.sh --no-build    # Skip Docker build
```

### 2. Gloas fork (Glamsterdam consensus layer) — ePBS (EIP-7732)

| Phase | Status | Detail |
|-------|--------|--------|
| 1. Types & Constants | DONE | 16 new types, BeaconBlockBody/BeaconState superstruct variants |
| 2. State Transitions | DONE | bid processing, PTC attestations, builder payments, withdrawals |
| 3. Fork Choice | DONE | 3-state payload model, all 8 reorg tests pass |
| 4. P2P Networking | DONE | gossip topics, validation, beacon processor integration |
| 5. Beacon Chain Integration | DONE | [docs/tasks/beacon-chain-integration.md](docs/tasks/beacon-chain-integration.md) |
| 6. Validator Client | DONE | [docs/tasks/validator-client.md](docs/tasks/validator-client.md) |
| 7. REST API | DONE | [docs/tasks/rest-api.md](docs/tasks/rest-api.md) |
| 8. Spec Tests | DONE | 78/78 + 138/138 passing, check_all_files_accessed passes |

**Phase 5:** DONE — external builder path implemented and verified with integration tests (bid selection, block production, self-build fallback)

**Phase 6:** DONE — all VC tasks complete (PTC duties, bid selection, duty discovery, external builder awareness)

**Phase 7:** DONE — blinded blocks endpoint verified: Gloas blocks have no payload to blind, BlindedPayload/FullPayload conversions are phantom-type pass-throughs of bid + payload_attestations

Reference:
- CL Specs: https://github.com/ethereum/consensus-specs/tree/master/specs/gloas
- ePBS spec: https://eips.ethereum.org/EIPS/eip-7732
### 3. Spec tests

[docs/tasks/spec-tests.md](docs/tasks/spec-tests.md) — 78/78 + 138/138 passing, check_all_files_accessed passes

### 4. ZK execution proofs (stateless validation) — DONE (stub proofs, SP1 infra ready)
[docs/tasks/zk-execution-proofs.md](docs/tasks/zk-execution-proofs.md) | [GitHub #28](https://github.com/dapplion/vibehouse/issues/28)
ZK proofs for execution payloads enabling stateless CL nodes. 20 tasks across 7 phases complete. Stateless devnet achieved finalized_epoch=9 with 3 proof-generators + 1 stateless node. SP1 Groth16 verifier integrated (`sp1-verifier` 6.0.1), guest/host programs built (`zkvm/`), async proof generation wired. Only 20f (real SP1 devnet) remains — requires SP1 toolchain + GPU.

### 5. More devnet testing

[docs/tasks/devnet-testing.md](docs/tasks/devnet-testing.md)

Current devnet only tests the happy path (4 homogeneous nodes, self-build, minimal preset). Run more scenarios:

- **Syncing** — DONE (script): `scripts/kurtosis-run.sh --sync` — 2 validators + 2 sync targets, catches up through Gloas fork boundary
- **Node churn** — DONE (script): `scripts/kurtosis-run.sh --churn` — kill validator node 4, verify chain continues (75% stake), restart, verify recovery
- **Mainnet preset** — DONE (script): `scripts/kurtosis-run.sh --mainnet` — 4 nodes, 512 validators, 32 slots/epoch, 12s slots
- **Long-running** — DONE (script): `scripts/kurtosis-run.sh --long` — epoch 50 target, periodic memory/CPU monitoring, ~40 min
- **Builder path** — DONE (script): `scripts/kurtosis-run.sh --builder` — genesis builder injection, proposer prefs + bid via lcli, chain health verified
- **Payload withholding** — DONE (script): `scripts/kurtosis-run.sh --withhold` — submit bid, no envelope, verify EMPTY path finalization
- **Network partitions** — DONE (script): `scripts/kurtosis-run.sh --partition` — stop 2/4 nodes (50% stake), verify finalization stalls, heal, verify recovery
- **Stateless + ZK** — DONE: proof-generators + stateless node (from priority 4)
- **Slashing scenarios** — DONE (script): `scripts/kurtosis-run.sh --slashings` — inject double-proposal and double-vote via lcli, verify slashed=true in state

### 6. ROCQ formal proofs for consensus-critical invariants — LOWEST PRIORITY
[GitHub #29](https://github.com/dapplion/vibehouse/issues/29) — Dead last. Only after everything else is done.

### 8. Backlog

- **Peer scoring** — DONE (Gloas ePBS topics: ExecutionBid, ExecutionPayload, PayloadAttestation, ExecutionProof)
- **ExecutionPayloadEnvelopesByRoot RPC** — DONE: `execution_payload_envelopes_by_root/1` P2P protocol, serves envelopes from store by block root, Gloas-only, MAX_REQUEST_PAYLOADS=128
- **Test coverage** — in progress (21 Gloas fork choice unit tests + 17 bid processing unit tests + 17 envelope processing unit tests + 15 withdrawal processing unit tests + 15 builder pending payments epoch processing unit tests + 21 state upgrade unit tests + 13 block replayer unit tests + 27 builder deposit processing unit tests + 20 payload attestation & PTC committee unit tests + 18 PtcDutiesMap unit tests + 15 attestation participation flag unit tests + 42 gossip verification integration tests + 9 genesis initialization unit tests + 7 expected withdrawals phase unit tests + 6 per_slot_processing payload availability unit tests + 6 proposer slashing payment removal unit tests + 4 same-slot attestation weight unit tests + 10 verify_attestation committee index tests + 8 proto_array viability tests + 6 attestation signing payload_present tests + 11 fork choice node state transition tests + 13 signature set construction tests + 17 ForkChoice wrapper method tests + 5 Builder::is_active tests + 14 verify_committee_index Gloas attestation validation tests + 8 gossip verification error path tests + 6 envelope signature verification tests + 11 per-block Gloas orchestration + fork dispatch tests + 8 process_proposer_lookahead tests + 22 execution_payload_header Gloas conversion tests + 10 execution_payload Gloas SSZ/accessor tests + 18 payload Gloas Full/Blinded type tests + 42 DataColumnSidecar Gloas variant tests + 32 BuilderBid Gloas type tests + 35 BeaconBlockBody Gloas variant tests + 13 SignedBeaconBlock Gloas blinding/conversion tests + 9 engine API Gloas type & NewPayloadRequest tests + 21 PubsubMessage Gloas gossip encode/decode tests + 75 ePBS Gloas type behavioral tests + 25 BeaconStateGloas unit tests + 22 ChainSpec Gloas timing/domain/scheduling tests + 15 ForkName Gloas unit tests + 11 ProposerPreferences tests + 8 SignedProposerPreferences tests + 11 BuilderPendingPayment tests + 11 BuilderPendingWithdrawal tests + 4 PtcDutyData API serde tests + 10 ExecutionBidPool edge case tests + 6 ObservedExecutionBids edge case tests + 9 ObservedPayloadAttestations edge case tests + 14 Gloas beacon_chain integration tests + 7 attestation index validation tests + 15 should_extend_payload/get_payload_tiebreaker tests + 16 get_gloas_weight/should_apply_proposer_boost_gloas tests + 18 compute_filtered_roots/get_ancestor_gloas/is_supporting_vote_gloas/get_gloas_children tests + 12 find_head_gloas proposer boost/gloas_head_payload_status tests + 16 BeaconChain Gloas method integration tests + 8 import_payload_attestation_message integration tests + 9 Gloas HTTP API integration tests + 10 envelope/body type tests + 6 HTTP API envelope POST/pre-Gloas guard tests + 12 HTTP API proposer_lookahead/PTC duties/attestation/withdrawal tests + 24 SSE event & API type tests + 5 fork choice state verification tests + 5 execution proof chain-dependent integration tests + 6 process_epoch_single_pass Gloas integration tests + 6 block production payload attestation packing tests + 2 dependent root stability proof tests + 2 store cold state dual-indexing tests + 3 block verification bid/DA bypass tests + 7 envelope processing integration tests + 3 engine API Gloas wire format tests + 7 stateless validation execution proof threshold tests + 9 gossip verification error path tests + 16 Gloas slot timing tests + 7 execution payload path integration tests + 13 fork choice Gloas method integration tests + 6 validator store Gloas signing domain tests + 5 fork choice attestation import integration tests + 5 fork transition boundary integration tests + 5 apply_execution_bid_to_fork_choice integration tests + 6 network gossip handler integration tests + 8 proposer preferences gossip handler tests + 6 block verification Gloas edge case tests + 5 attestation production payload_present tests + 6 gossip execution payload envelope handler tests + 6 execution proof gossip handler tests + 5 early attester cache Gloas payload_present tests + 8 self-build envelope EL/error path tests + 4 canonical_head/payload attributes Gloas branch tests + 6 execution bid gossip builder-path tests + 3 payload attestation gossip handler tests + 3 proposer preferences bid validation tests + 3 external builder block import lifecycle tests + 5 POST envelope error path/payload attestation data HTTP tests + 1 duplicate proposer preferences gossip test + 3 self-build envelope EL response tests + 5 withdrawal edge case tests + 4 multi-epoch chain health integration tests + 4 HTTP API builder bid submission tests + 3 envelope edge case integration tests + 3 sign_proposer_preferences validator store tests + 3 load_parent/range sync integration tests + 3 gossip envelope/load_parent EMPTY/historical attestation tests + 3 get_advanced_hot_state envelope re-application tests + 3 external builder envelope gossip verification tests + 3 pool/fork-choice field behavior tests + 3 gossip envelope EL error path/cross-epoch withdrawal tests + 3 canonical head head_hash fallback tests + 3 proposer boost timing/payload invalidation tests + 3 gossip verification/execution status lifecycle tests + 3 fork transition boundary/envelope error path tests + 3 bid/attestation equivocation and builder balance tests + 3 proposer preferences bid validation tests + 3 payload attestation gossip validation tests + 3 bid gossip validation tests + 3 bid signature/envelope finalization tests + 5 fork choice queued attestation dequeue/tiebreaker/multi-hop ancestor tests + 5 state processing error path tests + 5 BuilderPubkeyCache unit tests + 5 EMPTY parent/is_parent_block_full/withdrawal cap tests + 7 EarlyAttesterCache Gloas payload_present tests + 4 update_builder_pubkey_cache tests + 3 withdrawal OOB builder_index/nonzero index tests + 2 minimum active balance quorum tests + 5 range sync/multi-epoch/payload attestation data integration tests + 5 broadcast_proposer_preferences VC tests added)
- **Gloas slot timing** — DONE (#8686: SlotClock uses 4 intervals/slot after Gloas fork, fork choice proposer boost timing fixed, BN/VC wired)
- **CI workflow** — DONE (`.github/workflows/ci.yml`: check, ef-tests, unit-tests, fork-specific-tests)

---

## decision framework

When deciding what to work on next:

1. **Security fixes** — drop everything, write our own fix from spec
2. **Broken CI / failing tests** — fix before anything else
3. **Spec test failures** — consensus correctness is non-negotiable
4. **New spec changes in gloas** — track the spec closely
5. **Community-reported bugs** — real users, real issues
6. **Community feature requests with >3 upvotes** — the people have spoken
7. **Coverage improvements** — always be testing
9. **Code cleanup** — only when it unblocks other work

---

## work process

### documentation-driven development

Every piece of work must be tracked in a task document under `docs/tasks/`. This is non-negotiable.

- **`PLAN.md`** (this file) — master plan, priorities, references to task docs
- **`docs/tasks/`** — one file per task/workstream with objective, progress, blockers, decisions
- **`CLAUDE.md`** — repo-specific instructions for the claude loop

**The rule**: if you did work, it must be reflected in the task document. Every commit that changes code should update the relevant task doc.

### commit style

- lowercase, human-readable messages
- no conventional commits, no prefixes
- each commit atomic — one logical change
- never commit code that doesn't compile

### branch strategy

- `main` — always compiles, tests pass
- feature branches as needed, named descriptively

---

## key commands

```bash
# Rust environment
export CARGO_HOME=/home/openclaw-sigp/.openclaw/.cargo
export RUSTUP_HOME=/home/openclaw-sigp/.openclaw/.rustup
export PATH=/home/openclaw-sigp/.openclaw/.cargo/bin:$PATH

# Build + test
cargo build --release
RUST_MIN_STACK=8388608 cargo test --release -p ef_tests --features "ef_tests" --test "tests"

# Devnet
scripts/build-docker.sh                # Build Docker image
scripts/kurtosis-run.sh                # Full lifecycle
scripts/kurtosis-run.sh --no-build     # Skip build, reuse image
scripts/kurtosis-run.sh --no-teardown  # Leave enclave running
```

---

## non-goals

- Not trying to replace Lighthouse for production staking
- Not maintaining backwards compatibility with lighthouse release branches
- No conventional commits
