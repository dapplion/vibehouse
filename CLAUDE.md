# Vibehouse AI Assistant Guide

This file provides guidance for AI assistants (Claude Code, Codex, etc.) working with Vibehouse.

## Vibehouse-Specific

Vibehouse is a fork of [Lighthouse](https://github.com/sigp/lighthouse) from v8.0.1 (post-Fulu).

- **Read `plan.md` first** - it defines the work process, priorities, and the claude loop
- **Read `PROGRESS.md`** - it has the log of all work done so far
- **Read `docs/workstreams/`** - active workstream status and next steps
- All work must be tracked in committed markdown docs (see plan.md "documentation-driven development")
- Commit messages: lowercase, human-readable, no conventional commits
- Branch from `main` (not `unstable` - that's upstream's branch)
- Upstream remote: `sigp/lighthouse`, origin: `dapplion/vibehouse`

---

The rest of this file is inherited from upstream Lighthouse and still applies to code quality standards.

## Quick Reference

```bash
# Build
make install                              # Build and install Lighthouse
cargo build --release                     # Standard release build

# Test (prefer targeted tests when iterating)
cargo nextest run -p <package>            # Test specific package
cargo nextest run -p <package> <test>     # Run individual test
make test                                 # Full test suite (~20 min)

# Lint
make lint                                 # Run Clippy
cargo fmt --all && make lint-fix          # Format and fix
```

## Before You Start

Read the relevant guide for your task:

| Task | Read This First |
|------|-----------------|
| **Code review** | `.ai/CODE_REVIEW.md` |
| **Creating issues/PRs** | `.ai/ISSUES.md` |
| **Development patterns** | `.ai/DEVELOPMENT.md` |

## Critical Rules (consensus failures or crashes)

### 1. No Panics at Runtime

```rust
// NEVER
let value = option.unwrap();
let item = array[1];

// ALWAYS
let value = option?;
let item = array.get(1)?;
```

Only acceptable during startup for CLI/config validation.

### 2. Consensus Crate: Safe Math Only

In `consensus/` (excluding `types/`), use saturating or checked arithmetic:

```rust
// NEVER
let result = a + b;

// ALWAYS
let result = a.saturating_add(b);
```

## Important Rules (bugs or performance issues)

### 3. Never Block Async

```rust
// NEVER
async fn handler() { expensive_computation(); }

// ALWAYS
async fn handler() {
    tokio::task::spawn_blocking(|| expensive_computation()).await?;
}
```

### 4. Lock Ordering

Document lock ordering to avoid deadlocks. See [`canonical_head.rs:9-32`](beacon_node/beacon_chain/src/canonical_head.rs) for the pattern.

### 5. Rayon Thread Pools

Use scoped rayon pools from beacon processor, not global pool. Global pool causes CPU oversubscription when beacon processor has allocated all CPUs.

## Good Practices

### 6. TODOs Need Issues

All `TODO` comments must link to a GitHub issue.

### 7. Clear Variable Names

Avoid ambiguous abbreviations (`bb`, `bl`). Use `beacon_block`, `blob`.

## Branch & PR Guidelines

- Branch from `unstable`, target `unstable` for PRs
- Run `cargo sort` when adding dependencies
- Run `make cli-local` when updating CLI flags

## Project Structure

```
beacon_node/           # Consensus client
  beacon_chain/        # State transition logic
  store/               # Database (hot/cold)
  network/             # P2P networking
  execution_layer/     # EL integration
validator_client/      # Validator duties
consensus/
  types/               # Core data structures
  fork_choice/         # Proto-array
```

See `.ai/DEVELOPMENT.md` for detailed architecture.

## Maintaining These Docs

**These AI docs should evolve based on real interactions.**

### After Code Reviews

If a developer corrects your review feedback or points out something you missed:
- Ask: "Should I update `.ai/CODE_REVIEW.md` with this lesson?"
- Add to the "Common Review Patterns" or create a new "Lessons Learned" entry
- Include: what went wrong, what the feedback was, what to do differently

### After PR/Issue Creation

If a developer refines your PR description or issue format:
- Ask: "Should I update `.ai/ISSUES.md` to capture this?"
- Document the preferred style or format

### After Development Work

If you learn something about the codebase architecture or patterns:
- Ask: "Should I update `.ai/DEVELOPMENT.md` with this?"
- Add to relevant section or create new patterns

### Format for Lessons

```markdown
### Lesson: [Brief Title]

**Context:** [What task were you doing?]
**Issue:** [What went wrong or was corrected?]
**Learning:** [What to do differently next time]
```

### When NOT to Update

- Minor preference differences (not worth documenting)
- One-off edge cases unlikely to recur
- Already covered by existing documentation

## Devnet Testing (Kurtosis)

### Quick Commands

```bash
# Build Docker image (fast — uses host cargo cache, ~30s incremental)
scripts/build-docker.sh

# Run full devnet lifecycle (build + start + assertoor check + teardown)
scripts/kurtosis-run.sh

# Skip build (reuse existing vibehouse:local image)
scripts/kurtosis-run.sh --no-build

# Leave enclave running after test (for manual inspection)
scripts/kurtosis-run.sh --no-teardown
```

### How the Devnet Works

- Minimal preset: 8 slots/epoch, 6s/slot
- Gloas (ePBS) fork at epoch 1
- Single node: vibehouse CL + geth EL
- Assertoor automatically checks finalization, block proposals, and sync status
- 5-minute timeout — if assertoor hasn't passed by then, the run fails

### Bot Workflow

1. Make code change
2. Run `scripts/kurtosis-run.sh`
3. On failure: read dump logs in `/tmp/kurtosis-dump-*` (check CL logs first, then EL)
4. Fix the issue
5. Repeat

### Common Issues

- **Fork transition failures**: Check CL logs around epoch 1 boundary (slot 8)
- **Self-build envelope errors**: Check `process_self_build_envelope` and `get_execution_payload` paths
- **Engine API failures**: Check EL logs for `newPayload` / `forkchoiceUpdated` errors
- **Stale head hash**: Gloas uses fork choice head_hash, not `state.latest_block_hash()`

### Rules

- **NEVER** use the main `Dockerfile` for dev builds — it does a full Rust rebuild in Docker (5-10 min)
- **NEVER** run `kurtosis run` directly — old enclaves accumulate and waste resources
- **ALWAYS** use `scripts/kurtosis-run.sh` — it handles cleanup, assertoor polling, and timeout
- **ALWAYS** use `scripts/build-docker.sh` — it builds on host with incremental cargo cache
