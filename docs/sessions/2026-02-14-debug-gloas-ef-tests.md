# Gloas EF Tests Debugging Tracker

## Status: 11 failures remaining (66 passed)

Last run: `RUST_MIN_STACK=8388608 cargo test --release -p ef_tests --features "ef_tests" --test "tests"`

### Pre-existing (not Gloas, ignore):
- `fork_choice_get_head`: altair `voting_source_beyond_two_epoch` (pre-existing)
- `fork_choice_on_block`: altair `justified_update_always_if_better`, `justified_update_not_realized_finality` (pre-existing)

### Gloas-specific failures (9 test functions, ~35 individual cases):

## Methodology

For each failing test:
1. Understand exactly what the test does (read test vectors, understand the spec operation)
2. Look at affected code paths and the spec implementation
3. Register the precise error message and trace it to a root cause
4. Write a hypothesis and validate it
5. Fix and verify

---

## 1. operations_attestation (2/62 fail)

### Failing tests:
- `invalid_same_slot_attestation_index_one` - DidntFail
- `invalid_attestation_data_index_not_zero` - DidntFail

### What these tests do:
Both tests submit attestations with `data.index != 0`. In Gloas, the spec changes the constraint on `data.index` from `== 0` (Electra) to `< 2` (Gloas allows index 0 or 1 for PTC attestations). However these specific tests provide index values that should be invalid:
- `invalid_attestation_data_index_not_zero`: likely index >= 2
- `invalid_same_slot_attestation_index_one`: index == 1 but in a context where it should fail (same-slot attestation rule)

### Affected code:
- `consensus/state_processing/src/per_block_processing/verify_attestation.rs` - the `data.index` validation
- `consensus/state_processing/src/per_block_processing/process_operations.rs` - process_attestation
- `consensus/state_processing/src/common/get_attestation_participation.rs` - the `is_attestation_same_slot` check

### Hypothesis 1: Missing index validation for Gloas
In Electra, `data.index` must be 0. In Gloas, `data.index` must be < 2 (spec: `assert data.index in (0, 1)`). But the test `invalid_attestation_data_index_not_zero` probably has index >= 2. Need to check whether there's a `data.index < 2` check for Gloas.

The `is_attestation_same_slot` function in `get_attestation_participation.rs` was modified for Gloas to allow index 0 or 1. But the question is: does the code properly reject index >= 2?

### Hypothesis 2: Same-slot attestation with index 1
`invalid_same_slot_attestation_index_one`: in Gloas, same-slot attestations (PTC attestations) must have `data.index == 1`. Regular attestations have `data.index == 0`. This test likely tries to submit an index=1 attestation in a non-same-slot context, and it should be rejected.

### TODO:
- [ ] Read verify_attestation.rs to check if there's a Gloas-specific `data.index` validation
- [ ] Check `is_attestation_same_slot` logic for Gloas
- [ ] Check the EIP7732 spec for exact attestation index constraints

---

## 2. operations_execution_payload_bid (3/17 fail)

### Failing tests:
- `process_execution_payload_bid_valid_builder` - NotEqual
- `process_execution_payload_bid_sufficient_balance_with_pending_withdrawals` - NotEqual
- `process_execution_payload_bid_sufficient_balance_with_pending_payments` - NotEqual

### What these tests do:
Process an execution payload bid from a valid builder. The bid should modify:
- `latest_execution_payload_bid` in the state
- `builder_pending_payments` (record payment to builder)

### Affected code:
- `consensus/state_processing/src/per_block_processing/gloas.rs` - `process_execution_payload_bid`

### TODO:
- [ ] Get exact field mismatches from test output
- [ ] Read process_execution_payload_bid implementation
- [ ] Compare with spec

---

## 3. operations_payload_attestation (5/11 fail)

### Failing tests:
- `process_payload_attestation_partial_participation` - NotEqual
- `process_payload_attestation_uses_multiple_committees` - NotEqual
- `process_payload_attestation_payload_not_present` - NotEqual
- `process_payload_attestation_sampling_not_capped` - NotEqual
- `process_payload_attestation_payload_present` - NotEqual

### What these tests do:
Process payload attestations (PTC votes on whether the execution payload was present). These update builder pending payments and execution_payload_availability.

### Affected code:
- `consensus/state_processing/src/per_block_processing/gloas.rs` - `process_payload_attestation`

### TODO:
- [ ] Get exact field mismatches
- [ ] Read process_payload_attestation implementation
- [ ] Compare with spec

---

## 4. operations_proposer_slashing (3/38 fail)

### Failing tests:
- `builder_payment_deletion_current_epoch_first_slot` - NotEqual
- `builder_payment_deletion_current_epoch` - NotEqual
- `builder_payment_deletion_current_epoch_last_slot` - NotEqual

### What these tests do:
When a proposer is slashed, their builder pending payments for the current epoch should be deleted (set to default). The spec adds to `process_proposer_slashing`:
```python
# [New in Gloas:EIP7732] Delete builder pending payments for current epoch
current_epoch = get_current_epoch(state)
for slot_index in range(current_epoch * SLOTS_PER_EPOCH, (current_epoch + 1) * SLOTS_PER_EPOCH):
    payment_index = slot_index % len(state.builder_pending_payments)
    if state.proposer_lookahead[slot_index % SLOTS_PER_EPOCH] == proposer_index:
        state.builder_pending_payments[payment_index] = BuilderPendingPayment()
```

### Affected code:
- `consensus/state_processing/src/per_block_processing/process_operations.rs` - `process_proposer_slashings`
- `consensus/state_processing/src/common/slash_validator.rs` - `slash_validator`

### Hypothesis: Missing builder payment deletion logic
The current `process_proposer_slashings` and `slash_validator` don't have Gloas-specific builder payment deletion code. Need to add it either in `process_proposer_slashings` or after the `slash_validator` call.

### TODO:
- [ ] Add builder payment deletion in process_proposer_slashings for Gloas
- [ ] Determine exact location (proposer_slashings vs slash_validator)

---

## 5. operations_withdrawals (17/82 fail)

### Failing tests:
**DidntFail (2):**
- `invalid_builder_index_sweep` - should reject but doesn't
- `invalid_builder_index_pending` - should reject but doesn't

**NotEqual (15):** Various withdrawal processing mismatches including builder withdrawal processing, sweep spillover, mixed scenarios.

### What these tests do:
Gloas changes withdrawals significantly:
- Validators withdrawals are computed via `get_expected_withdrawals` which returns `(withdrawals, partial_withdrawals_count, builder_withdrawal_count)`
- Builder withdrawals are processed alongside validator withdrawals
- `next_withdrawal_builder_index` tracks the builder sweep position
- The block's `payload_expected_withdrawals` must match `get_expected_withdrawals`

### Affected code:
- `consensus/state_processing/src/per_block_processing/gloas.rs` - `process_withdrawals`, `get_expected_withdrawals`

### Hypothesis 1: Invalid builder index validation missing
The `invalid_builder_index_*` tests expect failures when `payload_expected_withdrawals` contains incorrect builder withdrawal info. Need validation that the block's withdrawals match expected.

### Hypothesis 2: get_expected_withdrawals computation wrong
The NotEqual failures suggest the withdrawal computation itself has bugs.

### TODO:
- [ ] Get exact field mismatches for one of the NotEqual cases
- [ ] Read get_expected_withdrawals implementation carefully
- [ ] Compare with spec
- [ ] Check builder index validation

---

## 6. operations_execution_payload_blinded + operations_execution_payload_full

### Error: Panic at handler.rs:102 - "test should load"

### What happens:
Gloas `execution_payload` test vectors contain `signed_envelope.ssz_snappy` (not `body.ssz_snappy`). The tests try to load a `BeaconBlockBody` from `body.ssz_snappy` which doesn't exist in Gloas test cases. These tests need a completely different handler for Gloas since the execution payload is delivered separately via `SignedExecutionPayloadEnvelope`.

### Hypothesis:
These tests should either:
1. Be disabled for Gloas (since execution_payload processing is fundamentally different)
2. Have a new Gloas-specific handler that loads `signed_envelope.ssz_snappy`

### TODO:
- [ ] Check if there's a `body.ssz_snappy` in the Gloas execution_payload test dir (there isn't)
- [ ] Either disable for Gloas or create new handler
- [ ] The Gloas execution_payload tests probably test `process_execution_payload_envelope`

---

## 7. sanity_blocks - withdrawal_success_two_blocks (1/62 fail)

### Error: StateRootMismatch

### What this test does:
Processes two consecutive blocks that include withdrawals. The state root doesn't match after processing.

### Hypothesis:
Related to the withdrawals processing bugs (same root cause as operations_withdrawals failures).

### TODO:
- [ ] Fix withdrawals first, then recheck this test

---

## 8. fork_choice_reorg (4/8 fail)

### Failing tests:
- `include_votes_another_empty_chain_with_enough_ffg_votes_previous_epoch`
- `include_votes_another_empty_chain_without_enough_ffg_votes_current_epoch`
- `include_votes_another_empty_chain_with_enough_ffg_votes_current_epoch`
- `simple_attempted_reorg_without_enough_ffg_votes`

### What these tests do:
Fork choice tests involving chain reorgs with FFG (Casper) voting. The fork choice handler processes blocks and attestations and checks the resulting head.

### Hypothesis:
Likely related to the block processing bugs (withdrawals, attestations) causing state root mismatches which cascade into fork choice failures.

### TODO:
- [ ] Fix operations bugs first, then recheck these
- [ ] If still failing, investigate fork_choice-specific Gloas changes

---

## Priority order:
1. **withdrawals** (17 failures, likely cascading to sanity_blocks + fork_choice)
2. **attestation** (2 failures, simpler fix)
3. **proposer_slashing** (3 failures, add builder payment deletion)
4. **payload_attestation** (5 failures)
5. **execution_payload_bid** (3 failures)
6. **execution_payload_full/blinded** (handler issue)
7. **sanity_blocks** (likely fixed by withdrawals fix)
8. **fork_choice_reorg** (likely fixed by other fixes)
