# vibehouse plan

> the vibemap. not a roadmap. roadmaps have deadlines. vibemaps have directions.

## 🚨 CRITICAL: ALWAYS RUN TESTS BEFORE PUSHING 🚨

**`make test-ef`** — Run this EVERY TIME before pushing code!

---

## fork point

vibehouse forks from [Lighthouse v8.0.1](https://github.com/sigp/lighthouse/releases/tag/v8.0.1), the last stable release covering the Fulu mainnet fork. Everything before v8.0.1 is inherited. Everything after is vibes.

---

## priorities

### 1. 🔥 Kurtosis solo devnet — vibehouse only
[docs/tasks/kurtosis-devnet.md](docs/tasks/kurtosis-devnet.md) — **URGENT: get a kurtosis devnet running with only vibehouse (no other clients). This is the #1 priority. Everything else waits.**

### 2. Gloas fork (Glamsterdam consensus layer) — ePBS (EIP-7732)

| Phase | Status | Task doc |
|-------|--------|----------|
| 1. Types & Constants | ✅ COMPLETE | — |
| 2. State Transition | ✅ COMPLETE | — |
| 3. Fork Choice | ✅ COMPLETE | — |
| 4. P2P Networking | ✅ COMPLETE | — |
| 5. Beacon Chain Integration | 🚧 IN PROGRESS | [docs/tasks/beacon-chain-integration.md](docs/tasks/beacon-chain-integration.md) |
| 6. Validator Client | NOT STARTED | [docs/tasks/validator-client.md](docs/tasks/validator-client.md) |
| 7. REST API | NOT STARTED | [docs/tasks/rest-api.md](docs/tasks/rest-api.md) |
| 8. Testing | NOT STARTED | — |

Reference:
- CL Specs: https://github.com/ethereum/consensus-specs/tree/master/specs/gloas
- ePBS spec: https://eips.ethereum.org/EIPS/eip-7732
- Upstream WIP: [sigp/lighthouse#8806](https://github.com/sigp/lighthouse/pull/8806)

### 3. Spec tests
[docs/tasks/spec-tests.md](docs/tasks/spec-tests.md) — 78/78 passing

### 4. Testing coverage
[docs/tasks/testing-coverage.md](docs/tasks/testing-coverage.md) — NOT STARTED

### 5. Kurtosis / epbs-devnet-0
[docs/tasks/kurtosis-devnet.md](docs/tasks/kurtosis-devnet.md) — target Feb 18, 2026

### 6. Community features
Track via GitHub Issues. Triage, label, implement, test, merge.

### 7. Upstream sync
[docs/tasks/upstream-sync.md](docs/tasks/upstream-sync.md) — ONGOING

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

## non-goals

- Not trying to replace Lighthouse for production staking
- Not maintaining backwards compatibility with lighthouse release branches
- No conventional commits
