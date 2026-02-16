# Testing Coverage

## Objective
Massively increase test coverage. Track it. Make it visible. Make it go up.

## Status: NOT STARTED

### Tasks
- [ ] Install `cargo-llvm-cov` in CI
- [ ] Run baseline coverage measurement
- [ ] Set up codecov.io integration
- [ ] Add coverage badge to README
- [ ] Document baseline coverage per crate

### Coverage targets
| Crate | Baseline | Target |
|-------|----------|--------|
| `consensus/state_processing` | TBD | 80%+ |
| `beacon_node/beacon_chain` | TBD | 70%+ |
| `beacon_node/fork_choice` | TBD | 85%+ |
| `consensus/types` | TBD | 90%+ |
| `beacon_node/network` | TBD | 60%+ |

### Test writing priority
1. Consensus-critical code (state transitions, fork choice, block validation)
2. Serialization (SSZ encode/decode)
3. P2P validation (gossip validation, peer scoring)
4. API correctness
5. Integration (end-to-end)

## Progress log
(none yet)
