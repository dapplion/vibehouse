# vibehouse plan

> the vibemap. not a roadmap. roadmaps have deadlines. vibemaps have directions.

## fork point

vibehouse forks from [Lighthouse v8.0.1](https://github.com/sigp/lighthouse/releases/tag/v8.0.1), the last stable release covering the Fulu mainnet fork (epoch 411,392, Dec 3 2025). Everything before v8.0.1 is inherited. Everything after is vibes.

---

## work process: the claude loop

A Claude instance runs continuously on this repo. This section defines how it operates.

### documentation-driven development

Every piece of work must be tracked in committed markdown documents. This is non-negotiable.

- **`PROGRESS.md`** - Living log of all work done. Every work session appends entries. Committed after every meaningful chunk of work.
- **`plan.md`** (this file) - The master plan. Checkboxes get checked off and committed as work completes.
- **`docs/workstreams/`** - One markdown file per active workstream (e.g., `gloas-implementation.md`, `spec-tests.md`, `coverage.md`). Contains detailed status, blockers, decisions made, and next steps.
- **`docs/bugs/`** - One file per non-trivial bug investigation. Contains reproduction steps, root cause analysis, and fix description.
- **`docs/features/`** - One file per community feature. Contains requirements, design, implementation notes.

**The rule**: If you did work, it must be reflected in a committed document before moving to the next task. Every commit that changes code should be preceded or accompanied by a doc commit explaining what and why. The repo's markdown files are the source of truth for project status.

### daily loop

The claude instance cycles through these phases repeatedly:

**Phase 1: Sync & Awareness**
1. `git fetch upstream` - check for new commits on upstream `unstable` and `stable`
2. Check [ethereum/consensus-specs](https://github.com/ethereum/consensus-specs) for new commits, PRs, or releases
3. Check [ethereum/consensus-spec-tests](https://github.com/ethereum/consensus-spec-tests) for new test releases
4. Check vibehouse GitHub Issues for new community requests, bug reports, and feature asks
5. Review any open PRs on vibehouse that need attention
6. Update this plan.md with any new information discovered

**Phase 2: Upstream Cherry-picks**
1. Review new upstream commits since last sync
2. Identify security fixes - these get cherry-picked immediately, no questions asked
3. Identify bug fixes relevant to our fork - cherry-pick with a clean commit
4. Identify performance improvements - cherry-pick if they don't conflict with our changes
5. For conflicting changes, create a branch, resolve conflicts, test, then merge
6. Run `cargo check` and `cargo test` after every cherry-pick batch
7. If tests fail, fix the issue before moving on

**Phase 3: Spec Implementation**
1. Read the latest consensus-specs gloas directory for any changes
2. Compare spec against current vibehouse implementation
3. Identify gaps - new features, changed behavior, removed fields
4. Implement changes in order of dependency (types first, then state transitions, then fork choice, then networking, then API)
5. Write tests for every spec change implemented
6. Run spec tests to validate
7. Each logical change gets its own commit with a clear message

**Phase 4: Testing & Coverage**
1. Run full test suite: `cargo test --workspace`
2. Run spec tests for all forks
3. Check coverage numbers - identify any regressions
4. Write new tests for uncovered code paths, prioritizing critical consensus logic
5. If kurtosis is set up, trigger a multi-client testnet run

**Phase 5: Community & Issues**
1. Check GitHub Issues for new requests
2. Triage: label as `community-request`, `bug`, `enhancement`, or `question`
3. Respond to questions with helpful answers
4. For feature requests with traction: implement, test, PR
5. For bugs: reproduce, fix, test, commit

**Phase 6: Documentation & Commit**
1. Update PROGRESS.md with everything done in this cycle
2. Update plan.md - check off completed items, add new items discovered
3. Update relevant workstream docs in `docs/workstreams/`
4. Commit all doc changes: `git add docs/ PROGRESS.md plan.md && git commit -m "progress update"`
5. Update README if any new features are worth highlighting
6. Clean up any stale branches
7. Ensure CI is green
8. Push all changes

### decision framework

When deciding what to work on next, use this priority order:

1. **Security fixes from upstream** - drop everything
2. **Broken CI / failing tests** - fix before anything else
3. **Spec test failures** - consensus correctness is non-negotiable
4. **New spec changes in gloas** - track the spec as closely as possible
5. **Community-reported bugs** - real users hitting real issues
6. **Community feature requests with >3 upvotes** - the people have spoken
7. **Coverage improvements** - always be testing
8. **Upstream cherry-picks (non-security)** - stay current
9. **Code cleanup and refactoring** - only when it unblocks other work
10. **Nice-to-have features** - vibes permitting

### commit style

- lowercase, human-readable messages
- no conventional commits, no prefixes like `feat:` or `fix:`
- examples: `implement gloas beacon state`, `cherry-pick upstream gossip fix`, `add tests for payload attestation`
- each commit should be atomic - one logical change per commit
- never commit code that doesn't compile

### branch strategy

- `main` - always compiles, tests pass, this is the "it works" branch
- `gloas-dev` - active gloas implementation work, may be broken
- `upstream-sync` - staging branch for upstream cherry-picks before merging to main
- feature branches as needed for larger changes, named descriptively

---

## priority 1: gloas fork (glamsterdam consensus layer)

The next Ethereum hard fork is **Glamsterdam** (execution: Amsterdam, consensus: Gloas). This is the single highest priority for vibehouse.

### Key EIPs to implement

- **EIP-7732: Enshrined Proposer-Builder Separation (ePBS)** - The big one. Separates block proposing from block building at the protocol level. Includes builder bids, execution payloads as separate messages, and payload attestations.
- **EIP-7916** - Data availability changes for gloas (DataColumnSidecar changes, removal of signed_block_header and kzg_commitments_inclusion_proof fields).
- **EIP-8016** - Progressive data structures.

### Implementation steps (detailed)

#### Step 1: Types & Constants
- [ ] Add gloas fork version constant and epoch placeholder
- [ ] Define new gloas types: `ExecutionBid`, `SignedExecutionBid`, `PayloadAttestation`, `PayloadAttestationMessage`
- [ ] Update `BeaconState` with gloas-specific fields (builder-related state)
- [ ] Update `BeaconBlockBody` for gloas (ePBS changes - payload becomes optional in proposer block, builder submits separately)
- [ ] Update `DataColumnSidecar` - remove `signed_block_header` and `kzg_commitments_inclusion_proof` fields
- [ ] Add gloas fork to the `ForkName` enum and all fork-conditional logic
- [ ] Update SSZ type definitions and serialization

#### Step 2: State Transition
- [ ] Implement `process_block` changes for gloas (ePBS block processing)
- [ ] Implement `process_execution_bid` - validate and apply builder bids
- [ ] Implement `process_payload_attestation` - handle payload attestation messages
- [ ] Update `process_epoch` for any gloas epoch processing changes
- [ ] Update `process_slots` to handle gloas fork transition
- [ ] Implement proposer/builder role separation in block processing
- [ ] Implement bid validation: check builder balance, bid amount, commitment validity

#### Step 3: Fork Choice
- [ ] Implement ePBS fork choice rule changes
- [ ] Update `on_block` to handle the proposer/builder split
- [ ] Implement `on_execution_bid` fork choice handler
- [ ] Implement `on_payload_attestation` fork choice handler
- [ ] Handle the case where payload is not revealed (builder withholding)
- [ ] Update equivocation detection for the new message types
- [ ] Test fork choice across fork boundary (fulu -> gloas transition)

#### Step 4: P2P Networking
- [ ] Add new gossip topics: `execution_bid`, `execution_payload`, `payload_attestation`
- [ ] Implement gossip validation for each new topic
- [ ] Update `DataColumnSidecar` gossip validation (no more header/inclusion proof checks, use kzg_commitments hash against builder bid instead)
- [ ] Update req/resp protocols if needed for gloas
- [ ] Handle gossip topic subscription/unsubscription at fork boundary
- [ ] Update peer scoring for new message types

#### Step 5: Beacon Chain Integration
- [ ] Wire up new gloas types through the beacon chain crate
- [ ] Update block import pipeline for ePBS (proposer block vs builder payload)
- [ ] Update fork choice store integration
- [ ] Handle the two-phase block: proposer commits, builder reveals
- [ ] Implement payload timeliness committee logic
- [ ] Update chain head tracking for ePBS

#### Step 6: Validator Client
- [ ] Update block proposal flow for ePBS (proposer creates block with bid selection)
- [ ] Implement bid selection logic (choose best bid from builders)
- [ ] Implement payload attestation duty (validators attest to payload presence)
- [ ] Update duty discovery for new gloas duties
- [ ] Handle the case where no bids are received

#### Step 7: REST API
- [ ] Add `/eth/v1/beacon/blinded_blocks` updates for ePBS
- [ ] Add execution bid submission endpoint
- [ ] Add payload attestation endpoint
- [ ] Update block retrieval endpoints to handle two-phase blocks
- [ ] Implement proposer lookahead endpoint
- [ ] Update SSE events for new gloas events (new bids, payload attestations)

#### Step 8: Testing
- [ ] Unit tests for every new type (serialization, deserialization, default values)
- [ ] Unit tests for every state transition function
- [ ] Integration tests for the full block processing pipeline
- [ ] Fork transition tests (fulu -> gloas)
- [ ] P2P tests for new gossip topics
- [ ] Spec test integration for gloas vectors
- [ ] Edge cases: missing payload, conflicting bids, late attestations

### Reference

- Upstream WIP: [sigp/lighthouse#8806 - Gloas payload processing](https://github.com/sigp/lighthouse/pull/8806)
- Upstream WIP: [sigp/lighthouse#8815 - Proposer lookahead](https://github.com/sigp/lighthouse/pull/8815)
- CL Specs: https://github.com/ethereum/consensus-specs/tree/master/specs/gloas
- EL-CL Engine API: https://github.com/ethereum/execution-apis/tree/main/src/engine (critical for ePBS - new engine methods for builder/proposer separation)
- ePBS spec: https://eips.ethereum.org/EIPS/eip-7732

---

## priority 2: spec tests

Run the latest version of consensus spec tests at all times. No excuses.

### How spec tests work in lighthouse

Lighthouse has an existing spec test runner in the `testing/` directory. It downloads test vectors from `ethereum/consensus-spec-tests` releases and runs them against the implementation. The test runner is organized by fork and test type.

### Steps (detailed)

- [ ] Audit current spec test runner - understand how it downloads, caches, and runs tests
- [ ] Check which spec test version is currently pinned in the codebase
- [ ] Update to the latest spec test release
- [ ] Ensure all existing fork tests pass (phase0 through fulu)
- [ ] Add gloas test scaffolding to the test runner
  - [ ] Register gloas fork in test runner fork mapping
  - [ ] Add gloas-specific test handlers (ePBS operations, payload attestation, bid processing)
  - [ ] Wire up new test types that don't exist in previous forks
- [ ] Set up CI job that:
  - [ ] Downloads latest spec test release
  - [ ] Runs all tests
  - [ ] Fails the build if any test fails
  - [ ] Reports which tests pass/fail/skip with counts
- [ ] Create automated PR bot that checks for new spec test releases daily
  - [ ] Compare current pinned version against latest GitHub release
  - [ ] If new version exists, create PR updating the pin
  - [ ] Run tests in the PR to catch regressions early
- [ ] Track spec test compatibility in a status badge on the README

### Test categories to cover

- `bls` - BLS signature operations
- `epoch_processing` - all epoch transition sub-functions
- `finality` - finality rule tests
- `fork` - fork transition tests (critical for gloas)
- `fork_choice` - fork choice rule tests (critical for ePBS fork choice)
- `genesis` - genesis state creation
- `light_client` - light client protocol
- `operations` - block operation processing (attestations, deposits, slashings, + new ePBS ops)
- `random` - randomized state transition tests
- `rewards` - reward/penalty calculation
- `sanity` - basic block/slot processing
- `ssz_static` - SSZ encoding/decoding for all types
- `transition` - multi-fork transition tests

---

## priority 3: testing coverage

Massively increase test coverage. Track it. Make it visible. Make it go up.

### Setup steps

- [ ] Install `cargo-llvm-cov` in CI environment
- [ ] Run baseline coverage measurement: `cargo llvm-cov --workspace --html`
- [ ] Upload coverage report as CI artifact
- [ ] Set up codecov.io integration (or equivalent)
  - [ ] Add codecov token to repo secrets
  - [ ] Add codecov upload step to CI
  - [ ] Add coverage badge to README
- [ ] Document baseline coverage per crate in a tracking table below

### Coverage targets by crate (fill in after baseline)

| Crate | Baseline | Target | Current |
|-------|----------|--------|---------|
| `consensus/state_processing` | TBD | 80%+ | TBD |
| `beacon_node/beacon_chain` | TBD | 70%+ | TBD |
| `beacon_node/fork_choice` | TBD | 85%+ | TBD |
| `consensus/types` | TBD | 90%+ | TBD |
| `beacon_node/network` | TBD | 60%+ | TBD |
| `beacon_node/http_api` | TBD | 70%+ | TBD |
| `validator_client` | TBD | 65%+ | TBD |
| `crypto/bls` | TBD | 95%+ | TBD |

### Test writing strategy

Priority order for writing new tests:

1. **Consensus-critical code** - state transitions, fork choice, block validation
   - Every function in `state_processing` should have unit tests
   - Every fork choice handler should have edge case tests
   - Block validation should test all rejection conditions
2. **Serialization** - SSZ encode/decode for all types, especially new gloas types
3. **P2P validation** - gossip validation functions, peer scoring edge cases
4. **API correctness** - REST API endpoint behavior matches spec
5. **Integration** - end-to-end block import, attestation processing, sync

### Coverage CI gate

- [ ] Add CI check: warn if PR decreases overall coverage by >0.5%
- [ ] Add CI check: fail if PR decreases consensus crate coverage at all
- [ ] Generate per-PR coverage diff report as comment on PRs

---

## priority 4: kurtosis multi-client testing

Run multi-client testnets in CI. If vibehouse can't interop, it doesn't ship.

### Setup steps (detailed)

#### Step 1: Local kurtosis setup
- [ ] Install kurtosis CLI in CI runner
- [ ] Fork/pin the [ethereum-package](https://github.com/ethpandaops/ethereum-package) kurtosis package
- [ ] Add vibehouse as a CL client option in the package configuration
- [ ] Build vibehouse docker image in CI (or use pre-built from earlier CI step)
- [ ] Test locally: vibehouse BN + vibehouse VC + geth EL

#### Step 2: Basic interop test
- [ ] Define a minimal kurtosis config:
  - 4 nodes total
  - vibehouse CL + geth EL (2 nodes)
  - prysm CL + geth EL (1 node)
  - lodestar CL + geth EL (1 node)
  - 32 validators split across nodes
- [ ] Run network for 3 epochs
- [ ] Assert: chain finalizes
- [ ] Assert: all nodes on same head
- [ ] Assert: no crashes or panics in vibehouse logs

#### Step 3: Comprehensive test scenarios
- [ ] **Finality test**: run for 10 epochs, verify finality at expected epochs
- [ ] **Deposit test**: submit deposits via deposit contract, verify activation
- [ ] **Voluntary exit test**: submit exits, verify processed
- [ ] **Slashing test**: create equivocating validators, verify slashing
- [ ] **Sync committee test**: verify sync committee participation across clients
- [ ] **Sync test**: start a node late, verify it syncs from peers
- [ ] **Restart test**: kill a vibehouse node, restart, verify it recovers

#### Step 4: Multi-EL interop
- [ ] Test vibehouse against each EL separately: geth, reth, nethermind, besu
- [ ] Create a mixed-EL config (all 4 ELs, all running vibehouse CL)
- [ ] Verify payload validation works correctly across all ELs

#### Step 5: CI Integration
- [ ] Run basic interop test on every PR (if CI resources allow)
- [ ] Run comprehensive tests nightly
- [ ] Run full multi-EL matrix weekly
- [ ] Collect and store test results for trend tracking
- [ ] Alert on regressions (test that used to pass now fails)

#### Step 6: Gloas devnet
- [ ] Once gloas implementation is ready, set up a gloas-fork kurtosis config
- [ ] Test ePBS flow: proposer creates block, builder submits bid, payload revealed
- [ ] Test across clients as other clients implement gloas
- [ ] Run gloas devnet continuously to catch regressions

### Alternative: lightweight interop testing

If kurtosis is too heavy for per-PR CI:

- [ ] Build a lightweight test harness using `lcli` (lighthouse CLI tool) and docker-compose
- [ ] Spin up 2-node network (vibehouse + one other CL) with geth EL
- [ ] Run for 2 epochs, check finality, tear down
- [ ] Target: under 5 minutes total

---

## priority 5: community features

The community asks, we build. Track requests via GitHub Issues.

### Triage process

When a new issue comes in:

1. Read the issue carefully
2. Label it:
   - `community-request` - someone wants a feature
   - `bug` - something is broken
   - `question` - someone needs help
   - `enhancement` - improvement to existing behavior
   - `good-first-issue` - suitable for new contributors
3. Respond within the current work cycle acknowledging the issue
4. For bugs: attempt to reproduce, ask for details if needed
5. For feature requests: assess feasibility, estimate effort, check if upstream has it

### Implementation process for community features

1. Create a branch: `community/{issue-number}-{short-description}`
2. Implement the feature
3. Write tests
4. Update docs if needed
5. Open PR referencing the issue
6. Merge when CI passes

### Current candidate features

From upstream PRs and community signals:

- [ ] Proposer lookahead endpoint ([upstream #8815](https://github.com/sigp/lighthouse/pull/8815))
  - Allows validators to know upcoming proposer duties further in advance
  - Cherry-pick from upstream once merged, or implement independently
- [ ] Improved HTTP client user-agent headers ([upstream #8786](https://github.com/sigp/lighthouse/pull/8786))
  - Better identification in network requests
  - Small change, easy cherry-pick
- [ ] Better process health observability ([upstream #8793](https://github.com/sigp/lighthouse/pull/8793))
  - Correctly compute process times during health observation
  - Important for operators monitoring their nodes
- [ ] Remove deprecated merge transition code
  - The merge happened years ago, this code is dead weight
  - Reduces binary size and compilation time
  - Upstream is working on this too
- [ ] Enhanced logging and metrics
  - Better structured logging for debugging
  - Additional prometheus metrics for operational visibility
- [ ] Whatever the community wants next - keep the issue tracker open and welcoming

---

## priority 6: upstream sync

Stay current with upstream lighthouse fixes and improvements.

### Detailed process

#### Regular sync (per work cycle)

1. `git fetch upstream`
2. `git log main..upstream/unstable --oneline` - see what's new
3. For each new commit, categorize:
   - **Security fix**: cherry-pick immediately to main
   - **Bug fix**: cherry-pick to `upstream-sync` branch
   - **Feature**: evaluate if relevant, cherry-pick if so
   - **Refactor**: cherry-pick if it doesn't conflict with our changes
   - **CI/tooling**: cherry-pick if useful
   - **Irrelevant**: skip
4. Test the `upstream-sync` branch: `cargo test --workspace`
5. If tests pass, merge `upstream-sync` into `main`
6. If conflicts: resolve carefully, test again
7. Push and verify CI

#### Handling divergence

As vibehouse accumulates its own changes, merging upstream will get harder. Strategy:

- Keep our changes well-isolated in clearly marked modules/files where possible
- Prefer adding new files over modifying existing ones when feasible
- When we must modify upstream files, add clear comments marking our changes
- Periodically rebase our changes on top of latest upstream to keep the diff clean
- If upstream implements something we've already done, prefer their implementation (less maintenance) unless ours is better

#### Tracking upstream PRs of interest

Maintain a watchlist of upstream PRs that we care about:

- [ ] [#8806 - Gloas payload processing](https://github.com/sigp/lighthouse/pull/8806)
- [ ] [#8815 - Proposer lookahead](https://github.com/sigp/lighthouse/pull/8815)
- [ ] [#8807 - Inactivity scores ef tests](https://github.com/sigp/lighthouse/pull/8807)
- [ ] [#8793 - Process health observation](https://github.com/sigp/lighthouse/pull/8793)
- [ ] [#8786 - HTTP client user-agent](https://github.com/sigp/lighthouse/pull/8786)
- [ ] Any PR labeled `security` or `critical`

---

## non-goals

- We are not trying to replace Lighthouse. Use Lighthouse for production staking.
- We are not maintaining backwards compatibility with lighthouse release branches.
- We don't do conventional commits. We write commit messages like humans.

---

## how to help

1. Pick any unchecked item above
2. Open a PR
3. We review fast
4. We merge fast
5. vibes
