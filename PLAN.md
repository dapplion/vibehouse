# vibehouse plan

> the vibemap. not a roadmap. roadmaps have deadlines. vibemaps have directions.

## fork point

vibehouse forks from [Lighthouse v8.0.1](https://github.com/sigp/lighthouse/releases/tag/v8.0.1), the last stable release covering the Fulu mainnet fork. Everything before v8.0.1 is inherited. Everything after is vibes.

---

## priorities

### 1. Kurtosis solo devnet — vibehouse only

Get a kurtosis devnet running with only vibehouse (no other clients).

- Config: minimal preset, gloas fork at epoch 1, single node (vibehouse CL + geth EL)
- Assertoor checks finalization, block proposals, sync status (5-min timeout)
- Consensus specs: v1.7.0-alpha.2
- Reference: https://notes.ethereum.org/@ethpandaops/epbs-devnet-0

**Steps:**
1. Build Docker image: `scripts/build-docker.sh` (host cargo build + Dockerfile.dev, ~30s incremental)
2. Run devnet: `scripts/kurtosis-run.sh` (clean enclave, start, poll assertoor, teardown)
3. Fix failures: check CL logs first, then EL in `/tmp/kurtosis-dump-*`

**Unstaged files ready to commit:**
- `Dockerfile.dev` — minimal Ubuntu image, copies pre-built binary
- `scripts/build-docker.sh` — builds on host with cargo cache
- `scripts/kurtosis-run.sh` — full lifecycle with assertoor polling
- `CLAUDE.md` — devnet testing docs section
- `kurtosis/vibehouse-epbs.yaml` — added assertoor + spamoor services

**Environment:**
- Docker installed, `openclaw` user in docker group (use `sg docker "cmd"`)
- Kurtosis v1.15.2 installed, engine running

### 2. Gloas fork (Glamsterdam consensus layer) — ePBS (EIP-7732)

| Phase | Status | Detail |
|-------|--------|--------|
| 1. Types & Constants | DONE | 16 new types, BeaconBlockBody/BeaconState superstruct variants |
| 2. State Transitions | DONE | bid processing, PTC attestations, builder payments, withdrawals |
| 3. Fork Choice | DONE | 3-state payload model, all 8 reorg tests pass |
| 4. P2P Networking | DONE | gossip topics, validation, beacon processor integration |
| 5. Beacon Chain Integration | 95% | [docs/tasks/beacon-chain-integration.md](docs/tasks/beacon-chain-integration.md) |
| 6. Validator Client | IN PROGRESS | [docs/tasks/validator-client.md](docs/tasks/validator-client.md) |
| 7. REST API | IN PROGRESS | [docs/tasks/rest-api.md](docs/tasks/rest-api.md) |
| 8. Spec Tests | DONE | 78/78 + 136/136 passing, check_all_files_accessed passes |

**Phase 5 remaining:** external builder path, ProposerPreferences topic (neither needed for self-build devnet)

**Phase 6 remaining:** block proposal flow with bid selection, fallback when no bids received

**Phase 7 remaining:** blinded blocks endpoint, bid submission endpoint, proposer lookahead

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
- **CI spec test job** — not set up

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
