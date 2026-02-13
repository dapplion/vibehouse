# workstream: test coverage

> status: **not started** | priority: 3

## overview

Set up coverage tracking, establish baselines, and systematically increase coverage for consensus-critical code.

## tools

- `cargo-llvm-cov` - preferred, uses LLVM instrumentation
- `cargo-tarpaulin` - alternative, simpler setup
- codecov.io or coveralls - for reporting and PR integration

## next steps

1. Install coverage tooling
2. Run baseline measurement
3. Identify critical low-coverage crates
4. Set up CI integration

## log

- 2026-02-13: workstream created
