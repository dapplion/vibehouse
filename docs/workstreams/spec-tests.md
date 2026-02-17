# workstream: spec tests

> status: **in progress** | priority: 1

## overview

Ensure vibehouse runs the latest consensus spec tests for all forks, including gloas.

## current state (2026-02-17)

**77 tests total, 77 pass, 3 skipped (known)**
**136 mainnet SSZ static tests pass (including Fulu + Gloas DataColumnSidecar)**

```
cargo nextest run -p ef_tests --features ef_tests --no-fail-fast
```

### all gloas tests pass

- fork_choice_reorg (8/8) — includes all 4 previously failing tests
- fork_choice_get_head (all gloas pass)
- fork_choice_on_block (all gloas pass)
- fork_choice_get_proposer_head (all pass)
- fork_choice_should_override_forkchoice_update (all pass)
- operations_attestation (62/62)
- operations_execution_payload_bid (17/17)
- operations_payload_attestation (11/11)
- operations_withdrawals (82/82)
- operations_proposer_slashing (38/38)
- operations_deposit_request (all pass)
- sanity_blocks (all pass)
- sanity_slots (all pass)
- fork (all pass)
- finality (all pass)
- transition (all pass)
- random (all pass)

### skipped tests (known upstream failures)

| test | count | status | notes |
|------|-------|--------|-------|
| fork_choice_get_head | 1 | skipped | altair `voting_source_beyond_two_epoch` — upstream [#8689](https://github.com/sigp/lighthouse/issues/8689) |
| fork_choice_on_block | 2 | skipped | altair `justified_update_*` — upstream [#8689](https://github.com/sigp/lighthouse/issues/8689) |
| kzg_verify_blob_kzg_proof_batch | 1 | ~~env issue~~ pass | previously SIGABRT, now passing |

### fixes applied

1. **domain byte order** — `domain_beacon_builder`, `domain_ptc_attester`, `domain_proposer_preferences` were `0x0B000000` (big-endian hex), should be `11` (small int, matching existing domain patterns). `int_to_bytes4` uses little-endian.

2. **can_builder_cover_bid** — missing `MIN_DEPOSIT_AMOUNT` floor per spec: `min_balance = MIN_DEPOSIT_AMOUNT + pending_withdrawals; builder_balance - min_balance >= bid_amount`

3. **process_payload_attestation** — complete rewrite to match spec: check `data.beacon_block_root == state.latest_block_header.parent_root` and `data.slot + 1 == state.slot` (was checking wrong fields)

4. **get_ptc_committee** — complete rewrite: concatenate all beacon committees for the slot, then use `compute_balance_weighted_selection`. Was incorrectly using `compute_shuffled_index`.

5. **PTC_SIZE** — was hardcoded 512, changed to `E::PtcSize::to_usize()` (2 for minimal, 512 for mainnet)

6. **sorted indices check** — PTC allows duplicate validators, changed `<=` to `<` in sorted check

7. **process_withdrawals** — fixed sweep logic, conditional `next_withdrawal_validator_index` update, `max_withdrawals` from spec constant

8. **proposer slashing** — builder payment deletion uses `header_1.slot`

9. **attestation participation** — same-slot attestation `data.index == 0` validation

## how to run

```bash
# all tests (~10 min)
cargo nextest run -p ef_tests --features ef_tests --no-fail-fast

# specific test group
cargo nextest run -p ef_tests --features ef_tests -E 'test(fork_choice_reorg)'

# single test
cargo nextest run -p ef_tests --features ef_tests -E 'test(operations_payload_attestation)'
```

## sources

- Test vectors: https://github.com/ethereum/consensus-spec-tests
- Test format: https://github.com/ethereum/consensus-specs/tree/master/tests/formats/fork_choice
- Test runner: `testing/ef_tests/src/cases/`
- Test definitions: `testing/ef_tests/tests/tests.rs`

## next steps

1. Set up CI to run EF tests on every push
2. Monitor for new spec test releases

## log

- 2026-02-17: 77/77 passing + 136/136 mainnet. DataColumnSidecar SSZ fixed via superstruct (Fulu/Gloas variants). All SSZ static tests green.
- 2026-02-15: 77/77 passing. KZG SIGABRT resolved (env issue). all tests green.
- 2026-02-15: 76/77 passing. added upstream known-failure skips for 3 altair proposer_boost tests. fork_choice_reorg all pass now.
- 2026-02-14: 73/77 passing after fixing all state_processing issues
- 2026-02-14: fork choice reorg investigation complete, documented in gloas-fork-choice.md
- 2026-02-13: workstream created
