# vibehouse progress log

> every work session gets an entry. newest first.

---

## 2026-02-14 - gloas spec research session 1

### what happened
- Read full gloas beacon-chain.md spec from consensus-specs repo
- Created `docs/workstreams/gloas-implementation.md` with detailed learnings
- Documented key ePBS concepts: builder registry, two-phase blocks, PTC, builder payments
- Set up hourly cron job to spawn vibehouse work agent
- Identified blockers: no Rust toolchain on host (can't compile/test yet)

### key learnings
- **Builder registry**: builders are separate from validators, use 0x03 withdrawal prefix
- **Two-phase blocks**: proposer commits to bid (phase 1), builder reveals payload (phase 2)
- **PTC (Payload Timeliness Committee)**: 512 validators attest to payload delivery
- **Builder payments**: quorum-based (60% stake), paid at epoch boundary if quorum met
- **State transition reordering**: withdrawals now before bid processing
- **Fork choice**: operates on beacon blocks; payload tracked separately via PTC attestations
- **Data availability**: DataColumnSidecar drops signed_block_header and inclusion_proof fields

### decisions made
- Document-first approach: write detailed workstream docs as I learn
- Use spec as ground truth, reference upstream PRs but verify against spec
- Track blockers explicitly (Rust toolchain missing on this host)
- Focus on research and documentation work until build environment ready

### next steps
- Continue reading other gloas specs (fork-choice, p2p, validator)
- Research spec test runner structure in lighthouse codebase
- Plan type hierarchy for gloas containers
- Check if build can happen in CI or different environment

---

## 2026-02-13 - project initialization

### what happened
- Forked lighthouse v8.0.1 as vibehouse
- Set up repository: `dapplion/vibehouse` on GitHub
- Added upstream remote pointing to `sigp/lighthouse`
- Rewrote README with vibehouse branding, ASCII banner, and SVG banner
- Created `plan.md` with six priorities: gloas, spec tests, coverage, kurtosis, community, upstream sync
- Defined the "claude loop" - the work process for the 24/7 Claude instance
- Set up docs directory structure for workstream tracking

### research done
- Reviewed lighthouse v8.0.1 release (Fulu mainnet fork, Dec 3 2025)
- Reviewed upstream lighthouse open PRs - 77 open, active gloas work in progress
- Researched Glamsterdam/Gloas fork: EIP-7732 (ePBS), EIP-7916, EIP-8016
- Found upstream gloas WIP: PR #8806 (payload processing), PR #8815 (proposer lookahead)
- Identified consensus-specs gloas directory and spec test structure
- Identified Engine API specs for EL-CL communication

### decisions made
- Fork point: v8.0.1 (not v8.1.0 - we want the clean Fulu release as base)
- Branch strategy: main (stable), gloas-dev (wip), upstream-sync (cherry-picks)
- Documentation-driven: all work tracked in committed markdown
- Priority order: security > tests > spec > community > upstream > cleanup

### next steps
- Run `cargo check` to verify the build works
- Run existing test suite to establish baseline
- Begin auditing the spec test runner
- Start reading gloas consensus-specs in detail
- Set up CI workflows
