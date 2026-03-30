# Trust Assumptions

This document tracks all axioms used in vibehouse ROCQ proofs. Every axiom is a point where the proof trusts something it cannot verify mechanically.

## Current Axioms

### A1: BLS Signatures (not modeled)

BLS signature verification is axiomatized as a boolean predicate with standard correctness properties. We do not model the cryptographic internals. This is consistent with all prior Ethereum formal verification work (Runtime Verification Gasper proofs, ConsenSys Dafny, Galois blst verification).

### A2: Hash Functions (not modeled)

Hash functions (SHA-256, SSZ tree hashing) are treated as opaque functions. We assume collision resistance but do not prove it.

### A3: Rust Integer Semantics

We model Rust `u64` arithmetic as natural numbers with overflow checks. The proofs assume that `safe_arith` operations (`safe_add`, `safe_div`, `safe_mul`, `safe_rem`) correctly implement checked arithmetic — i.e., they return errors on overflow rather than wrapping. This is verified by Rust's type system and the `safe_arith` crate's test suite, not by the ROCQ proofs.

## Verification Scope

The proofs target:
1. **Tier 2**: PTC quorum arithmetic, builder payment slot indexing, payment sweep logic
2. **Tier 1**: Fork choice — head selection viability, best-child weight ordering, pruning safety (parent order + weight/viability preservation), Gloas payload status exclusivity/transitions, should_extend_payload characterization, reorg resistance, comparison totality
3. **Tier 3**: Execution payload envelope state transition — availability/payment index bounds, state mutation frame (bid fields preserved, payment blanking, block hash update, withdrawal queueing), header caching idempotence, verification completeness (all 8 consistency checks are necessary), availability index injectivity
