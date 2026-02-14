# workstream: spec tests

> status: **in progress** | priority: 1

## overview

Ensure vibehouse runs the latest consensus spec tests for all forks, including gloas.

## current state (2026-02-14)

**77 tests total, 73 pass, 4 fail**

```
cargo nextest run -p ef_tests --features ef_tests --no-fail-fast
```

### passing gloas tests

All state_processing tests pass:
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

### failing tests

| test | count | status | notes |
|------|-------|--------|-------|
| fork_choice_get_head | 1 | pre-existing | altair `voting_source_beyond_two_epoch` |
| fork_choice_on_block | 2 | pre-existing | altair `justified_update_*` |
| fork_choice_reorg | 4 | **gloas** | needs fork choice rewrite, see `gloas-fork-choice.md` |
| kzg_verify_blob_kzg_proof_batch | 1 | env issue | SIGABRT, likely KZG library/env |

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

1. Fix fork_choice_reorg failures (see `gloas-fork-choice.md`)
2. Investigate kzg SIGABRT
3. Investigate pre-existing altair fork_choice failures

## log

- 2026-02-14: 73/77 passing after fixing all state_processing issues
- 2026-02-14: fork choice reorg investigation complete, documented in gloas-fork-choice.md
- 2026-02-13: workstream created
