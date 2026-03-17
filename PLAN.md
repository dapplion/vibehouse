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

[docs/tasks/spec-tests.md](docs/tasks/spec-tests.md) — DONE: 79/79 + 139/139 passing, check_all_files_accessed passes, spec tracked to v1.7.0-alpha.3, automated release check workflow deployed

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

### 6. Code review & quality improvement — DONE
[docs/tasks/code-review-quality.md](docs/tasks/code-review-quality.md)

All 5 phases complete: (1) clippy/doc/dead-code/unwrap audit, (2) architecture review (superstruct, duplication, error types, pub visibility, module organization), (3) correctness deep-dive (spec conformance, constants, edge cases), (4) performance (clone/allocation audit, DB patterns, serialization), (5) test quality (~600+ Gloas tests, strong assertions, no flakiness). Fixes shipped: GnosisEthSpec MaxPayloadAttestations U2→U4, envelope error type wrapping, 2 pub→pub(crate) downgrades, withdrawal processing optimization, cargo doc warnings.

### 7. ROCQ formal proofs for consensus-critical invariants — LOWEST PRIORITY
[GitHub #29](https://github.com/dapplion/vibehouse/issues/29) — Dead last. Only after everything else is done.

### 8. Backlog

- **Peer scoring** — DONE (Gloas ePBS topics: ExecutionBid, ExecutionPayload, PayloadAttestation, ExecutionProof)
- **ExecutionPayloadEnvelopesByRoot RPC** — DONE: `execution_payload_envelopes_by_root/1` P2P protocol, serves envelopes from store by block root, Gloas-only, MAX_REQUEST_PAYLOADS=128
- **Test coverage** — DONE (unit tests at diminishing returns as of run 1463). ~2000+ tests added across runs 1376-1463 covering: Gloas ePBS types, fork choice, state processing, gossip verification, beacon chain integration, HTTP API, network/RPC protocol, validator client, operation pool, store/hdiff, crypto, slasher, and common utilities. Remaining untested code requires complex integration harnesses (beacon_chain.rs, block_verification.rs, sync modules, system health). See `docs/tasks/code-review-quality.md` for the full audit.
- **Gloas slot timing** — DONE (#8686: SlotClock uses 4 intervals/slot after Gloas fork, fork choice proposer boost timing fixed, BN/VC wired)
- **CI workflow** — DONE (`.github/workflows/ci.yml`: check, ef-tests, unit-tests, beacon-chain-tests, http-api-tests, network+op-pool-tests (parallelized in run 248); `nightly-tests.yml`: all prior forks beacon_chain/network/op_pool + http_api electra/fulu)
- **Rebranding** — DONE: binary renamed, all crate names renamed, LighthouseSubcommands→VibehouseSubcommands, eth2 feature flag lighthouse→vibehouse, all modules/functions/variables/constants/string literals/comments/API paths renamed, `lighthouse/` source directory renamed to `vibehouse/`, TLS test fixtures updated, Docker symlinks removed, book documentation fully rebranded (61 files, api_lighthouse.md→api_vibehouse.md)
- **Sync: NoPeer graceful handling** — DONE (#33): range sync leaves batches in AwaitingDownload on NoPeer instead of faking download failures; retried via resume() when peers join
- **Sync: custody column robustness** — DONE (#32): empty custody column responses now count as download failures (bounded by MAX_CUSTODY_COLUMN_DOWNLOAD_ATTEMPTS)
- **Deterministic test crypto** — DONE (#36 partial): SecretKey/Signature TestRandom implementations now use deterministic RNG via interop keypairs KDF
- **Sync: peer group tracking** — DONE (#34): batch downloads track PeerGroup instead of single PeerId; block and column requests decoupled — blocks download immediately, columns deferred until custody peers available

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
