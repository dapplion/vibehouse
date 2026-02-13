# workstream: spec tests

> status: **not started** | priority: 2

## overview

Ensure vibehouse runs the latest consensus spec tests for all forks, including gloas as it becomes available.

## current state

Lighthouse v8.0.1 has an existing spec test runner. Need to audit it and understand:
- Where spec test version is pinned
- How tests are downloaded and cached
- What forks are currently covered
- How to add new fork test support

## sources

- Test vectors: https://github.com/ethereum/consensus-spec-tests
- Test runner: `testing/` directory in the vibehouse codebase

## next steps

1. Audit `testing/` directory structure
2. Find the spec test version pin
3. Run existing tests and verify they pass
4. Check for latest spec test release and compare

## log

- 2026-02-13: workstream created
