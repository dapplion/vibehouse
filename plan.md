# vibehouse plan

> the vibemap. not a roadmap. roadmaps have deadlines. vibemaps have directions.

## fork point

vibehouse forks from [Lighthouse v8.0.1](https://github.com/sigp/lighthouse/releases/tag/v8.0.1), the last stable release covering the Fulu mainnet fork (epoch 411,392, Dec 3 2025). Everything before v8.0.1 is inherited. Everything after is vibes.

---

## priority 1: gloas fork (glamsterdam consensus layer)

The next Ethereum hard fork is **Glamsterdam** (execution: Amsterdam, consensus: Gloas). This is the single highest priority for vibehouse.

### Key EIPs to implement

- **EIP-7732: Enshrined Proposer-Builder Separation (ePBS)** - The big one. Separates block proposing from block building at the protocol level. Includes builder bids, execution payloads as separate messages, and payload attestations.
- **EIP-7916** - Data availability changes for gloas (DataColumnSidecar changes, removal of signed_block_header and kzg_commitments_inclusion_proof fields).
- **EIP-8016** - Progressive data structures.

### Implementation steps

- [ ] Track [ethereum/consensus-specs](https://github.com/ethereum/consensus-specs) `specs/gloas/` directory continuously
- [ ] Implement gloas beacon state changes
- [ ] Implement gloas block processing changes
- [ ] Implement ePBS fork choice rule changes
- [ ] Implement new gossip topics (execution_bid, execution_payload, payload_attestation)
- [ ] Update DataColumnSidecar for gloas (remove header + inclusion proof fields)
- [ ] Implement builder bid validation and processing
- [ ] Implement payload attestation logic
- [ ] Update P2P layer for gloas networking spec changes
- [ ] Update REST API for gloas-specific endpoints (proposer lookahead, etc.)

### Reference

- Upstream WIP: [sigp/lighthouse#8806 - Gloas payload processing](https://github.com/sigp/lighthouse/pull/8806)
- Specs: https://github.com/ethereum/consensus-specs/tree/master/specs/gloas

---

## priority 2: spec tests

Run the latest version of consensus spec tests at all times. No excuses.

### Steps

- [ ] Set up CI job to pull latest spec test releases from [ethereum/consensus-spec-tests](https://github.com/ethereum/consensus-spec-tests)
- [ ] Run full spec test suite on every PR (phase0, altair, bellatrix, capella, deneb, electra, fulu, gloas)
- [ ] Add spec test version pinning with automated update PRs
- [ ] Ensure gloas spec tests pass as they become available
- [ ] Monitor upstream spec test releases and update within 24h

---

## priority 3: testing coverage

Massively increase test coverage. Track it. Make it visible. Make it go up.

### Steps

- [ ] Set up `cargo-llvm-cov` or `cargo-tarpaulin` for coverage measurement
- [ ] Add coverage reporting to CI (upload to codecov or similar)
- [ ] Establish baseline coverage numbers per crate
- [ ] Identify lowest-coverage critical crates and write tests for them
- [ ] Add coverage gate to PRs (no merges that decrease coverage without justification)
- [ ] Target: beacon_chain, fork_choice, state_processing, networking crates first

---

## priority 4: kurtosis multi-client testing

Run multi-client testnets in CI. If vibehouse can't interop, it doesn't ship.

### Steps

- [ ] Set up [Kurtosis](https://github.com/kurtosis-tech/kurtosis) Ethereum package in CI
- [ ] Run vibehouse as beacon node + validator against multiple EL clients (geth, reth, nethermind, besu)
- [ ] Run vibehouse alongside other CL clients (prysm, teku, nimbus, lodestar) for interop testing
- [ ] Create a standard test scenario: deposits, exits, slashings, sync committee duties
- [ ] Run kurtosis on every PR to main (or at minimum nightly + on release branches)
- [ ] Build dashboards to track interop test results over time
- [ ] Consider building a lightweight alternative to kurtosis for faster CI feedback

---

## priority 5: community features

The community asks, we build. Track requests via GitHub Issues.

### Process

- Community opens an issue with a feature request
- We label it `community-request`
- If it gets traction (upvotes, discussion), we prioritize it
- We ship it

### Candidate features (from upstream PRs and community discussion)

- [ ] Proposer lookahead endpoint ([upstream #8815](https://github.com/sigp/lighthouse/pull/8815))
- [ ] Improved HTTP client user-agent headers ([upstream #8786](https://github.com/sigp/lighthouse/pull/8786))
- [ ] Better process health observability ([upstream #8793](https://github.com/sigp/lighthouse/pull/8793))
- [ ] Remove deprecated merge transition code (cleanup)
- [ ] Whatever the community wants next

---

## priority 6: upstream sync

Stay current with upstream lighthouse fixes and improvements.

### Process

- [ ] Set up weekly upstream merge/cherry-pick routine
- [ ] Review upstream PRs that land on `unstable` for relevant fixes
- [ ] Backport security fixes immediately
- [ ] Maintain a clean merge history with upstream

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
