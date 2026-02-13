# vibehouse progress log

> every work session gets an entry. newest first.

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
