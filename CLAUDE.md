# Vibehouse AI Assistant Guide

This file provides guidance for AI assistants (Claude Code, Codex, etc.) working with Vibehouse.

## Vibehouse-Specific

Vibehouse is a fork of [Lighthouse](https://github.com/sigp/lighthouse) from v8.0.1 (post-Fulu).

- **Read `PLAN.md` first** - it defines the work process, priorities, and phase status
- **Read `docs/tasks/`** - active task docs with detailed progress logs
- **Read `docs/workstreams/`** - implementation design docs and reference
- All work must be tracked in committed markdown docs (see plan.md "documentation-driven development")
- Commit messages: lowercase, human-readable, no conventional commits
- Branch from `main` (not `unstable` - that's upstream's branch)
- Upstream remote: `sigp/lighthouse`, origin: `dapplion/vibehouse`

---

The rest of this file is inherited from upstream Lighthouse and still applies to code quality standards.

## Quick Reference

```bash
# Build
cargo build --release

# Lint
cargo fmt --all && make lint-fix
make lint
```

## Testing — Run the Right Tests for What You Changed

Do NOT run `make test-ef` or `make test-full` on every change. Run targeted tests based on which code you touched.

### consensus/types/ — Type definitions, SSZ, superstruct

```bash
cargo nextest run --release -p types
# Then: SSZ static tests to catch serialization regressions
cargo nextest run --release -p ef_tests --features "ef_tests,fake_crypto" ssz_static
```

### consensus/state_processing/ — State transitions, block/epoch processing

```bash
cargo nextest run --release -p state_processing
# Then: EF operations + epoch + sanity tests (fake_crypto = 2-3x faster)
cargo nextest run --release -p ef_tests --features "ef_tests,fake_crypto" -E 'test(/operations_|epoch_processing_|sanity_/)'
```

If you touched a specific operation (e.g. withdrawals), narrow further:
```bash
cargo nextest run --release -p ef_tests --features "ef_tests,fake_crypto" operations_withdrawals
```

### consensus/fork_choice/ or consensus/proto_array/ — Fork choice

```bash
cargo nextest run --release -p proto_array    # unit tests, fast (~1s)
cargo nextest run --release -p fork_choice
# Then: EF fork choice tests
cargo nextest run --release -p ef_tests --features "ef_tests" -E 'test(/fork_choice_/)'
```

### beacon_node/network/ — P2P, gossip, networking

```bash
# Run for gloas fork only (not all 8 forks)
env FORK_NAME=gloas cargo nextest run --release --features "fork_from_env" -p network
```

If your change is fork-agnostic, pick one recent fork to save time:
```bash
env FORK_NAME=fulu cargo nextest run --release --features "fork_from_env" -p network
```

### beacon_node/beacon_chain/ — Block import, chain management

```bash
# Run for the fork you're working on
env FORK_NAME=gloas cargo nextest run --release --features "fork_from_env,slasher/lmdb" -p beacon_chain
```

### beacon_node/http_api/ — REST API endpoints

```bash
env FORK_NAME=fulu cargo nextest run --release --features "beacon_chain/fork_from_env" -p http_api
```

### beacon_node/operation_pool/ — Attestation/op pool

```bash
env FORK_NAME=gloas cargo nextest run --release --features "beacon_chain/fork_from_env" -p operation_pool
```

### validator_client/ — Validator duties, signing

```bash
cargo nextest run --release -p validator_client
```

### Before pushing — verify nothing is broken across the workspace

```bash
# Quick workspace check (excludes heavy packages — ~2 min)
cargo nextest run --workspace --release \
  --exclude ef_tests --exclude beacon_chain --exclude slasher --exclude network --exclude http_api
```

### Full EF spec tests — only when touching consensus-critical code

```bash
# Real crypto + fake crypto + check_all_files_accessed (the full suite)
make run-ef-tests
```

### Tips

- `fake_crypto` skips BLS verification — 2-3x faster, good for logic-only changes
- `FORK_NAME=gloas` runs tests for one fork only — 8x faster than testing all forks
- `cargo nextest run -p ef_tests --features "ef_tests,fake_crypto" <filter>` — filter is a substring match on test name
- Filter with `-E 'test(/pattern/)'` for regex matching on test names
- All tests require `--release` for reasonable performance

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
