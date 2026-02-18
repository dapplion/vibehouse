# vibehouse plan

> the vibemap. not a roadmap. roadmaps have deadlines. vibemaps have directions.

## fork point

vibehouse forks from [Lighthouse v8.0.1](https://github.com/sigp/lighthouse/releases/tag/v8.0.1), the last stable release covering the Fulu mainnet fork. Everything before v8.0.1 is inherited. Everything after is vibes.

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
| 5. Beacon Chain Integration | DONE (self-build) | [docs/tasks/beacon-chain-integration.md](docs/tasks/beacon-chain-integration.md) |
| 6. Validator Client | IN PROGRESS | [docs/tasks/validator-client.md](docs/tasks/validator-client.md) |
| 7. REST API | DONE | [docs/tasks/rest-api.md](docs/tasks/rest-api.md) |
| 8. Spec Tests | DONE | 78/78 + 136/136 passing, check_all_files_accessed passes |

**Phase 5 remaining:** external builder path, ProposerPreferences topic (neither needed for self-build devnet)

**Phase 6 remaining:** block proposal flow with bid selection, fallback when no bids received

**Phase 7 remaining:** blinded blocks endpoint verified working (Gloas blocks have no payload to blind, conversion is phantom-type only)

Reference:
- CL Specs: https://github.com/ethereum/consensus-specs/tree/master/specs/gloas
- ePBS spec: https://eips.ethereum.org/EIPS/eip-7732
- Upstream WIP: [sigp/lighthouse#8806](https://github.com/sigp/lighthouse/pull/8806)

### 3. Spec tests

[docs/tasks/spec-tests.md](docs/tasks/spec-tests.md) — 78/78 + 136/136 passing, check_all_files_accessed passes

### 4. Upstream sync

[docs/tasks/upstream-sync.md](docs/tasks/upstream-sync.md) — monitoring upstream PRs, cherry-picking non-gloas fixes

### 5. Backlog

- **Peer scoring** — design complete, not yet implemented (functional at defaults)
- **Test coverage tooling** — not started
- **CI workflow** — DONE (`.github/workflows/ci.yml`: check, ef-tests, unit-tests, fork-specific-tests)

---

## decision framework

When deciding what to work on next:

1. **Security fixes from upstream** — drop everything
2. **Broken CI / failing tests** — fix before anything else
3. **Spec test failures** — consensus correctness is non-negotiable
4. **New spec changes in gloas** — track the spec closely
5. **Community-reported bugs** — real users, real issues
6. **Community feature requests with >3 upvotes** — the people have spoken
7. **Coverage improvements** — always be testing
8. **Upstream cherry-picks (non-security)** — stay current
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
